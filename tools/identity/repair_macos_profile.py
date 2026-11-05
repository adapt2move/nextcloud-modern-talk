#!/usr/bin/env python3
"""Repair only a changed mount device in a legacy NextCloud Modern Talk profile.

Default mode checks without writing. --repair takes the app's exclusive profile
lock, backs up the nonsecret metadata locally, and changes only meta.root after
rechecking the same user, directory inode and profile marker. No credential item,
account disposition, generation or pending operation is read or modified.

This preserves v0.0.8 compatibility for the current mount. A newer application
with persistent volume bindings is needed to avoid repeating it after a remount.
"""
import argparse
from contextlib import ExitStack, closing
import fcntl
import os
from pathlib import Path
import re
import sqlite3
import stat
import subprocess
import sys
import time

MAX_DATABASE = 4 * 1024 * 1024
MARKER = re.compile(r"[0-9a-f]{32}")
LEGACY = re.compile(r"unix:([0-9]+):([0-9]+):([0-9]+)")
MUTATION = re.compile(r"\b(write|write_data|append|add_file|add_subdirectory|delete|delete_child|writeattr|writeextattr|writesecurity|chown)\b")


class RepairError(Exception):
    """An allowlisted diagnostic, never a raw filesystem/database error."""


def check_acl(path, private):
    if sys.platform != "darwin":
        return
    result = subprocess.run(["/bin/ls", "-lde", str(path)], capture_output=True,
                            timeout=3, check=False)
    if result.returncode or len(result.stdout) > 32768:
        raise RepairError("Zugriffsrechte konnten nicht geprüft werden.")
    for line in result.stdout.decode("utf-8", errors="replace").splitlines()[1:]:
        if re.match(r"\s*\d+:.*\ballow\b", line) and (private or MUTATION.search(line)):
            raise RepairError("Zusätzliche Zugriffsrechte verhindern die Reparatur.")


def check_directory(path):
    uid = os.geteuid()
    if not path.is_absolute() or any(part in (".", "..") for part in path.parts):
        raise RepairError("Der Speicherort muss ein absoluter lokaler Pfad sein.")
    if Path.home() not in path.parents or any(part in {"tmp", "Temp", "Dropbox", "OneDrive", "CloudStorage", "Mobile Documents"} for part in path.parts):
        raise RepairError("Der Speicherort muss im lokalen Benutzerordner liegen.")
    for node in [*reversed(path.parents), path]:
        info = node.lstat()
        private = node == path
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid not in (0, uid)
                or info.st_mode & 0o022
                or private and (info.st_uid != uid or stat.S_IMODE(info.st_mode) != 0o700)):
            raise RepairError("Der Datenordner hat ungeeignete Zugriffsrechte.")
        check_acl(node, private)
    return path.stat()


def open_private(stack, directory, path, name, writable=False):
    flags = os.O_RDWR if writable else os.O_RDONLY
    fd = os.open(name, flags | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                 dir_fd=directory)
    stack.callback(os.close, fd)
    info = os.fstat(fd)
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
            or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1):
        raise RepairError("Eine benötigte Datei hat ungeeignete Zugriffsrechte.")
    check_acl(path / name, True)
    return fd, info


def connect(path, writable=False):
    db = sqlite3.connect(path.as_uri() + ("?mode=rw" if writable else "?mode=ro"),
                         uri=True, timeout=0)
    deadline = time.monotonic() + 5
    db.set_progress_handler(lambda: time.monotonic() >= deadline, 1000)
    db.execute("PRAGMA trusted_schema=OFF")
    return db


def metadata(db, marker, identity):
    if db.execute("PRAGMA quick_check").fetchall() != [("ok",)]:
        raise RepairError("Die Kontodaten sind beschädigt; keine Änderung vorgenommen.")
    if db.execute("PRAGMA journal_mode").fetchone() != ("delete",):
        raise RepairError("Unerwartetes Datenbankformat; keine Änderung vorgenommen.")
    if db.execute("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger'").fetchone() != (0,):
        raise RepairError("Unerwartetes Datenbankformat; keine Änderung vorgenommen.")
    if db.execute("SELECT type FROM sqlite_master WHERE name='meta'").fetchall() != [("table",)]:
        raise RepairError("Unerwartetes Datenbankformat; keine Änderung vorgenommen.")
    rows = db.execute("SELECT schema,profile,store,root FROM meta LIMIT 2").fetchall()
    if (len(rows) != 1 or type(rows[0][0]) is not int or rows[0][0] not in (5, 6, 7)
            or rows[0][1] != marker or not isinstance(rows[0][2], str)
            or not MARKER.fullmatch(rows[0][2]) or not isinstance(rows[0][3], str)):
        raise RepairError("Profilmarker oder Datenformat passen nicht; keine Änderung vorgenommen.")
    schema, profile, store, root = rows[0]
    match = LEGACY.fullmatch(root)
    if len(root) > 128 or not match:
        raise RepairError("Kein unterstütztes altes Profil; keine Änderung vorgenommen.")
    uid, device, inode = map(int, match.groups())
    if uid >= 1 << 32 or device >= 1 << 64 or inode >= 1 << 64 or root != f"unix:{uid}:{device}:{inode}":
        raise RepairError("Die Ordnerkennung hat ein unerwartetes Format.")
    if uid != os.geteuid() or inode != identity.st_ino:
        raise RepairError("Benutzer oder Datenordner wurden geändert; keine Änderung vorgenommen.")
    return (schema, profile, store, root), f"unix:{uid}:{identity.st_dev}:{inode}"


def recover(path, repair=False):
    path = Path(path)
    identity = check_directory(path)
    with ExitStack() as stack:
        directory = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        stack.callback(os.close, directory)
        lock, _ = open_private(stack, directory, path, "profile.lock", writable=True)
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RepairError("Talk verwendet den Kontospeicher noch. Beende die App vollständig.") from None
        marker_fd, marker_info = open_private(stack, directory, path, "profile.id")
        if marker_info.st_size != 32:
            raise RepairError("Der Profilmarker ist ungültig; keine Änderung vorgenommen.")
        marker = os.read(marker_fd, 33).decode("ascii")
        if not MARKER.fullmatch(marker):
            raise RepairError("Der Profilmarker ist ungültig; keine Änderung vorgenommen.")
        _, database_info = open_private(stack, directory, path, "restoration.sqlite")
        if not 0 < database_info.st_size <= MAX_DATABASE:
            raise RepairError("Die Datenbankgröße ist unerwartet; keine Änderung vorgenommen.")
        for suffix in ("-journal", "-wal", "-shm"):
            name = "restoration.sqlite" + suffix
            if os.path.lexists(path / name):
                open_private(stack, directory, path, name)
        database = path / "restoration.sqlite"
        with closing(connect(database)) as source:
            before, current = metadata(source, marker, identity)
            if before[3] == current:
                return "ALREADY_MATCHES", None
            if not repair:
                return "DEVICE_CHANGED", None
            backup_name = f"restoration.before-binding-{time.time_ns()}.sqlite"
            backup_fd = os.open(backup_name, os.O_RDWR | os.O_CREAT | os.O_EXCL
                                | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=directory)
            stack.callback(os.close, backup_fd)
            check_acl(path / backup_name, True)
            deadline = time.monotonic() + 5

            def progress(_status, _remaining, _total):
                if time.monotonic() >= deadline:
                    raise RepairError("Die Sicherung hat zu lange gedauert; keine Änderung vorgenommen.")

            with closing(connect(path / backup_name, writable=True)) as backup:
                backup.execute("PRAGMA synchronous=EXTRA")
                backup.execute("PRAGMA fullfsync=ON")
                source.backup(backup, pages=64, progress=progress, sleep=0.01)
                if backup.execute("PRAGMA quick_check").fetchall() != [("ok",)]:
                    raise RepairError("Die Sicherung ist ungültig; keine Änderung vorgenommen.")
            os.fsync(backup_fd)
            os.fsync(directory)
        live = check_directory(path)
        if (live.st_dev, live.st_ino) != (identity.st_dev, identity.st_ino):
            raise RepairError("Der Datenordner hat sich inzwischen geändert; keine Änderung vorgenommen.")
        with closing(connect(database, writable=True)) as db:
            db.execute("PRAGMA synchronous=EXTRA")
            db.execute("PRAGMA fullfsync=ON")
            with db:
                db.execute("BEGIN IMMEDIATE")
                observed, observed_current = metadata(db, marker, identity)
                if observed != before or observed_current != current:
                    raise RepairError("Die Kontodaten haben sich inzwischen geändert; keine Änderung vorgenommen.")
                updated = db.execute("UPDATE meta SET root=? WHERE root=? AND profile=?",
                                     (current, before[3], marker))
                if updated.rowcount != 1:
                    raise RepairError("Die Zuordnung konnte nicht eindeutig aktualisiert werden.")
        return "REPAIRED", backup_name


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repair", action="store_true", help="Back up metadata and update the legacy mount association")
    parser.add_argument("--profile", type=Path, help="Use a different local app data folder")
    args = parser.parse_args()
    if sys.platform != "darwin":
        print("Dieses Werkzeug ist für macOS bestimmt.", file=sys.stderr)
        return 1
    path = args.profile or Path.home() / "Library/Application Support/org.adapt2move.nextcloud-modern-talk"
    try:
        state, backup = recover(path, args.repair)
    except RepairError as error:
        print(str(error), file=sys.stderr)
        return 1
    except (OSError, sqlite3.Error, UnicodeError, subprocess.SubprocessError):
        print("Der lokale Kontospeicher konnte nicht bearbeitet werden. Beende Talk und prüfe Speicherort und Zugriffsrechte.", file=sys.stderr)
        return 1
    if state == "DEVICE_CHANGED":
        print("Bestätigt: Nur die Datenträgernummer hat sich geändert. Mit --repair wird die Zuordnung nach einer Sicherung aktualisiert.")
    elif state == "ALREADY_MATCHES":
        print("Die Ordnerzuordnung passt bereits. Es wurde nichts geändert.")
    else:
        print("Die Ordnerzuordnung wurde aktualisiert. Du kannst Talk jetzt wieder starten.")
        print(f"Sicherung im App-Datenordner: {backup}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
