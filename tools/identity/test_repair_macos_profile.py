"""Synthetic metadata and POSIX-lock checks; not native macOS qualification."""
from contextlib import closing
import fcntl
import importlib.util
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("repair_profile", Path(__file__).with_name("repair_macos_profile.py"))
repair = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(repair)


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix=".nmt-binding-test-", dir=Path.home())
        self.addCleanup(self.temporary.cleanup)
        self.profile = Path(self.temporary.name)
        self.marker = "1" * 32
        for name, content in [("profile.id", self.marker), ("profile.lock", "")]:
            file = self.profile / name
            file.write_text(content)
            file.chmod(0o600)
        identity = self.profile.stat()
        self.current = f"unix:{os.geteuid()}:{identity.st_dev}:{identity.st_ino}"
        self.old = f"unix:{os.geteuid()}:{identity.st_dev + 1}:{identity.st_ino}"
        self.database = self.profile / "restoration.sqlite"
        with closing(sqlite3.connect(self.database)) as db:
            db.executescript("""CREATE TABLE meta(schema INTEGER,profile TEXT,store TEXT,root TEXT);
                CREATE TABLE accounts(id TEXT,disposition INTEGER,eligible TEXT);
                CREATE TABLE operations(id TEXT,settlement INTEGER,retained TEXT,cleanup INTEGER);
                INSERT INTO accounts VALUES('synthetic-account',1,NULL);
                INSERT INTO operations VALUES('retained',1,NULL,0),('pending',0,'retained',0);""")
            db.execute("INSERT INTO meta VALUES(7,?,?,?)", (self.marker, "2" * 32, self.old))
            db.commit()
        self.database.chmod(0o600)

    def read(self, name="restoration.sqlite"):
        with closing(sqlite3.connect(self.profile / name)) as db:
            return (db.execute("SELECT * FROM meta").fetchall(),
                    db.execute("SELECT * FROM accounts").fetchall(),
                    db.execute("SELECT * FROM operations").fetchall())

    def change(self, statement, values=()):
        with closing(sqlite3.connect(self.database)) as db:
            db.execute(statement, values)
            db.commit()

    def backups(self):
        return list(self.profile.glob("restoration.before-binding-*.sqlite"))

    def test_check_is_read_only_and_creates_no_backup(self):
        before = self.database.read_bytes()
        self.assertEqual(repair.recover(self.profile), ("DEVICE_CHANGED", None))
        self.assertEqual(before, self.database.read_bytes())
        self.assertEqual(self.backups(), [])

    def test_repair_backs_up_and_changes_only_root_while_preserving_unresolved_work(self):
        before = self.read()
        state, backup = repair.recover(self.profile, repair=True)
        self.assertEqual(state, "REPAIRED")
        self.assertEqual(self.read(backup), before)
        after = self.read()
        self.assertEqual(after[0], [(7, self.marker, "2" * 32, self.current)])
        self.assertEqual(after[1:], before[1:])
        self.assertEqual((self.profile / backup).stat().st_mode & 0o7777, 0o600)
        self.assertEqual(repair.recover(self.profile, repair=True), ("ALREADY_MATCHES", None))
        self.assertEqual(len(self.backups()), 1)

    def test_another_user_directory_profile_or_persistent_binding_is_rejected(self):
        identity = self.profile.stat()
        for root in [f"unix:{os.geteuid() + 1}:1:{identity.st_ino}",
                     f"unix:{os.geteuid()}:1:{identity.st_ino + 1}",
                     "macos:501:11111111-2222-3333-4444-555555555555:42"]:
            self.change("UPDATE meta SET root=?", (root,))
            before = self.read()
            with self.assertRaises(repair.RepairError):
                repair.recover(self.profile, repair=True)
            self.assertEqual(self.read(), before)
            self.assertEqual(self.backups(), [])
        self.change("UPDATE meta SET root=?,profile=?", (self.old, "3" * 32))
        with self.assertRaises(repair.RepairError):
            repair.recover(self.profile, repair=True)
        self.assertEqual(self.backups(), [])

    def test_an_active_owner_blocks_repair(self):
        with (self.profile / "profile.lock").open("r+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(repair.RepairError, "Talk verwendet"):
                repair.recover(self.profile, repair=True)
        self.assertEqual(self.read()[0][0][3], self.old)
        self.assertEqual(self.backups(), [])

    def test_unsafe_file_permissions_or_symlinks_are_rejected(self):
        self.database.chmod(0o644)
        with self.assertRaises(repair.RepairError):
            repair.recover(self.profile, repair=True)
        self.database.chmod(0o600)
        original = self.profile / "original.sqlite"
        self.database.rename(original)
        self.database.symlink_to(original.name)
        with self.assertRaises(OSError):
            repair.recover(self.profile, repair=True)
        self.assertEqual(self.backups(), [])

    def test_failed_backup_leaves_original_untouched(self):
        from unittest.mock import patch
        before = self.database.read_bytes()
        with patch.object(repair.os, "fsync", side_effect=OSError("synthetic failure")):
            with self.assertRaises(OSError):
                repair.recover(self.profile, repair=True)
        self.assertEqual(self.database.read_bytes(), before)

    def test_changed_metadata_after_backup_is_not_overwritten(self):
        from unittest.mock import patch
        original_connect = repair.connect

        def changed_before_write(path, writable=False):
            if writable and path == self.database:
                self.change("UPDATE meta SET store=?", ("3" * 32,))
            return original_connect(path, writable)

        with patch.object(repair, "connect", side_effect=changed_before_write):
            with self.assertRaisesRegex(repair.RepairError, "inzwischen geändert"):
                repair.recover(self.profile, repair=True)
        self.assertEqual(self.read()[0][0][2:], ("3" * 32, self.old))
        self.assertEqual(len(self.backups()), 1)

    def test_macos_acl_check_rejects_mutable_parents_and_any_private_allow_grant(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        for entry, private, allowed in [
            (" 0: group:everyone deny delete", False, True),
            (" 0: group:everyone allow read,readattr", False, True),
            (" 0: group:everyone allow add_file", False, False),
            (" 0: group:everyone allow add_subdirectory", False, False),
            (" 0: group:everyone allow read", True, False),
        ]:
            output = SimpleNamespace(returncode=0, stdout=("directory header\n" + entry).encode())
            with patch.object(repair.sys, "platform", "darwin"), patch.object(repair.subprocess, "run", return_value=output):
                if allowed:
                    repair.check_acl(self.profile, private)
                else:
                    with self.assertRaises(repair.RepairError):
                        repair.check_acl(self.profile, private)


if __name__ == "__main__":
    unittest.main()
