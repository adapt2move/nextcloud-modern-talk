#!/usr/bin/env python3
"""Write and inspect the DMG layout with isolated, hash-pinned build tools.

The two pure-Python wheels remain in a private build directory. A Python -I -S
child imports only those wheels and the standard library; no host installation,
dependency resolution or shipped application dependency is involved.
"""
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
from urllib.parse import unquote, urlsplit
import urllib.request

HERE = Path(__file__).resolve().parent
LOCK = HERE / "macos-layout-tools.json"
MAX_STORE = 2 * 1024 * 1024
MAX_ALIAS = 64 * 1024
MAX_ENTRIES = 256
MAX_REPLY = 128 * 1024
APP_NAME = "NextCloud Modern Talk.app"
POSITIONS = {APP_NAME: (204, 235), "Applications": (576, 235)}
WINDOW_OPTIONS = {
    "WindowBounds": "{{120, 120}, {780, 480}}",
    "SidebarWidth": 180,
    **dict.fromkeys(("ShowStatusBar", "ContainerShowSidebar", "PreviewPaneVisibility",
                     "ShowTabView", "ShowToolbar", "ShowPathbar", "ShowSidebar"), False),
}
ICON_OPTIONS = {
    "viewOptionsVersion": 1, "backgroundType": 2,
    "backgroundColorRed": 1.0, "backgroundColorGreen": 1.0, "backgroundColorBlue": 1.0,
    "gridOffsetX": 0.0, "gridOffsetY": 0.0, "gridSpacing": 100.0,
    "arrangeBy": "none", "showIconPreview": False, "showItemInfo": False,
    "labelOnBottom": True, "textSize": 13.0, "iconSize": 112.0,
    "scrollPositionX": 0.0, "scrollPositionY": 0.0,
}


def command(argv, timeout, payload=None, limit=MAX_REPLY):
    # These children do not launch other processes. subprocess.run kills and
    # reaps its child on timeout/interruption. The Swift compiler uses the
    # release builder's existing native-tool route and its own separate limit.
    result = subprocess.run(list(map(str, argv)), input=payload, capture_output=True,
                            timeout=timeout)
    if result.returncode:
        raise ValueError("Persisted DMG layout tool failed: "
                         + result.stderr.decode(errors="replace")[-2000:])
    if len(result.stdout) > limit:
        raise ValueError("Persisted DMG layout tool exceeded its output limit")
    return result.stdout


def locked_tools():
    return json.loads(LOCK.read_text())["wheels"]


def checked_wheels(directory):
    paths = []
    for tool in locked_tools():
        path = Path(directory) / tool["filename"]
        info = path.lstat()
        if not stat.S_ISREG(info.st_mode) or info.st_size != tool["size"]:
            raise ValueError("Invalid layout parser wheel: " + tool["filename"])
        if hashlib.sha256(path.read_bytes()).hexdigest() != tool["sha256"]:
            raise ValueError("Layout parser wheel checksum mismatch: " + tool["filename"])
        paths.append(str(path))
    return paths


def download_tools(directory):
    for tool in locked_tools():
        # The caller imposes a total 60-second child lifetime as well as this
        # socket timeout. Read at most the locked artifact length plus one byte.
        with urllib.request.urlopen(tool["url"], timeout=20) as response:
            data = response.read(tool["size"] + 1)
        if len(data) != tool["size"] or hashlib.sha256(data).hexdigest() != tool["sha256"]:
            raise ValueError("Layout parser download checksum/size mismatch: " + tool["filename"])
        (Path(directory) / tool["filename"]).write_bytes(data)
    checked_wheels(directory)


def prepare(temporary):
    wheels = Path(temporary) / "layout-tools"
    wheels.mkdir(mode=0o700)
    command([sys.executable, "-I", "-S", __file__, "--download", wheels], 60)
    resolver = wheels / "resolve-background"
    from macos_runtime import run
    run(["xcrun", "swiftc", HERE / "macos_layout.swift", "-o", resolver], 180)
    return wheels, resolver


def write(mount, tools):
    wheels, _ = tools
    command([sys.executable, "-I", "-S", __file__, "--write", wheels, mount], 20)


def write_layout(mount):
    """Use dmgbuild's public writer route on the builder's owned RW mount.

    https://github.com/dmgbuild/dmgbuild/blob/main/src/dmgbuild/core.py
    Create the alias after mounting so its volume/file identities are real.
    Finder is opened only later, on the delivered read-only image.
    """
    from ds_store import DSStore
    from mac_alias import Alias
    path = mount / ".DS_Store"
    background = mount / ".background/background.png"
    if os.statvfs(mount).f_flag & os.ST_RDONLY:
        raise ValueError("Cannot write the layout to a read-only mount")
    if (path.is_symlink() or (path.exists() and not path.is_file())
            or background.is_symlink() or not background.is_file()):
        raise ValueError("Invalid disk-image layout/background path")
    alias = Alias.for_file(str(background)).to_bytes()
    if not 0 < len(alias) <= MAX_ALIAS:
        raise ValueError("Generated background alias exceeds its limit")
    with DSStore.open(str(path), "w+") as store:
        store["."]["vSrn"] = ("long", 1)
        store["."]["bwsp"] = WINDOW_OPTIONS
        store["."]["icvp"] = {**ICON_OPTIONS, "backgroundImageAlias": alias}
        store["."]["icvl"] = ("type", b"icnv")
        for name, position in POSITIONS.items():
            store[name]["Iloc"] = position
    read_layout(path)


class StoreInput(io.BytesIO):
    """Bound library I/O, including its allocator's padding on short reads."""
    def __init__(self, data):
        super().__init__(data)
        self.size = len(data)
        self.remaining = 8 * MAX_STORE

    def read(self, size=-1):
        if size < 0 or size > self.size - self.tell() or size > self.remaining:
            raise ValueError("DS_Store read exceeds its extent/work limit")
        self.remaining -= size
        return super().read(size)

    def write(self, data):
        raise ValueError("DS_Store inspection is read-only")


def record_context(entries, options=None):
    # Include record/key names only, never alias bytes or decoded paths.
    records = [f"{name[:48]}:{code.decode('ascii', errors='replace')}"
               for name, code in entries][:8]
    keys = sorted(str(key)[:48] for key in options)[:16] if isinstance(options, dict) else []
    return json.dumps({"records": records,
                       "icvp_keys": keys}, ensure_ascii=True)[:1400]


def read_layout(path):
    # Import only inside the isolated parser child (or explicit parser tests).
    from ds_store import DSStore
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or not 0 < info.st_size <= MAX_STORE:
        raise ValueError("DS_Store is not a bounded regular file")
    with path.open("rb") as stream:
        data = stream.read(MAX_STORE + 1)
    if len(data) != info.st_size:
        raise ValueError("DS_Store changed while reading")
    entries = {}
    try:
        with DSStore.open(StoreInput(data), "r") as store:
            for entry in store:
                key = (entry.filename.casefold(), entry.code)
                if key in entries or len(entries) == MAX_ENTRIES:
                    raise ValueError("Duplicate or excessive DS_Store entries")
                entries[key] = entry.value
    except Exception as error:
        raise ValueError(f"DS_Store decoding failed after {len(entries)} records: "
                         f"{type(error).__name__}: {str(error)[:240]}; "
                         + record_context(entries)) from error
    options = entries.get((".", b"icvp"))
    context = record_context(entries, options)
    if not isinstance(options, dict):
        raise ValueError("Missing persisted icon-view options: " + context)
    if entries.get((".", b"icvl")) != b"icnv" or entries.get((".", b"vSrn")) != 1:
        raise ValueError("Persisted icon view changed; " + context)
    if entries.get((".", b"bwsp")) != WINDOW_OPTIONS:
        raise ValueError("Persisted window options changed; " + context)
    for key, expected in (("backgroundType", 2), ("iconSize", 112), ("textSize", 13)):
        if options.get(key) != expected:
            raise ValueError(f"Persisted {key} changed: {str(options.get(key))[:80]}; " + context)
    for name, expected in POSITIONS.items():
        if entries.get((name.casefold(), b"Iloc")) != expected:
            raise ValueError(f"Persisted position changed for {name}; " + context)
    alias = options.get("backgroundImageAlias")
    if not isinstance(alias, (bytes, bytearray)) or not 0 < len(alias) <= MAX_ALIAS:
        raise ValueError("Missing/invalid persisted background alias; " + context)
    return {"background_alias": base64.b64encode(alias).decode("ascii"),
            "store_sha256": hashlib.sha256(data).hexdigest(), "entries": len(entries)}


def inspect(mount, tools):
    wheels, resolver = tools
    parsed = json.loads(command([sys.executable, "-I", "-S", __file__, "--inspect",
                                 wheels, Path(mount) / ".DS_Store"], 20))
    alias = base64.b64decode(parsed["background_alias"], validate=True)
    if not 0 < len(alias) <= MAX_ALIAS:
        raise ValueError("Persisted background alias exceeds its limit")
    resolved = json.loads(command([resolver, mount], 30, alias, 16384))
    background_url = resolved.get("url")
    try:
        if not isinstance(background_url, str) or len(background_url) > 8192:
            raise ValueError("background URL exceeds its bound")
        url = urlsplit(background_url)
        if (url.scheme != "file" or url.netloc not in ("", "localhost")
                or url.query or url.fragment or not url.path.startswith("/")):
            raise ValueError("background is not a local file URL")
        actual = Path(unquote(url.path, errors="strict"))
        expected = Path(mount) / ".background/background.png"
        if expected.is_symlink() or not actual.samefile(expected):
            raise ValueError("background belongs to a different file")
    except (OSError, ValueError) as error:
        raise ValueError("Persisted background does not identify this disk image's background: "
                         + repr(str(background_url)[:512])) from error
    # A stale bookmark can still resolve correctly after a volume is remounted;
    # physical identity, not the stale advisory flag or a stored path, decides.
    return {"inspection": "DS_Store + CoreFoundation alias resolution",
            "store_sha256": parsed["store_sha256"], "entries": parsed["entries"],
            "alias_sha256": hashlib.sha256(alias).hexdigest(),
            "alias_stale": resolved["stale"],
            "tools_lock_sha256": hashlib.sha256(LOCK.read_bytes()).hexdigest()}


def main():
    try:
        if len(sys.argv) == 3 and sys.argv[1] == "--download":
            download_tools(Path(sys.argv[2]))
        elif len(sys.argv) == 4 and sys.argv[1] in ("--inspect", "--write"):
            sys.path[:0] = checked_wheels(Path(sys.argv[2]))
            if sys.argv[1] == "--write":
                write_layout(Path(sys.argv[3]))
            else:
                print(json.dumps(read_layout(Path(sys.argv[3])), sort_keys=True))
        else:
            raise ValueError("Expected --download directory, --write tools mount or --inspect tools DS_Store")
    except Exception as error:
        print(f"{type(error).__name__}: {str(error)[:3000]}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
