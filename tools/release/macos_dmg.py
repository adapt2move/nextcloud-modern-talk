#!/usr/bin/env python3
"""Native styled disk image, followed by read-only layout/code inspection."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import stat
import tempfile
from urllib.parse import unquote, urlsplit

from macos_runtime import inspect_bundle, run, sha

APP_NAME = "NextCloud Modern Talk.app"
VOLUME_NAME = "NextCloud Modern Talk"
ICON = "assets/macos/AppIcon.icns"
BACKGROUND = "assets/macos/dmg-background.png"


def artwork(root, stage, temporary):
    work = temporary / "artwork"
    work.mkdir()
    run(["xcrun", "swift", root / "tools/release/macos_artwork.swift", work,
         root / "apps/desktop/assets/fonts/GoogleSans-Regular.ttf"], 180)
    icons = work / "AppIcon.iconset"
    icons.mkdir()
    for points in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            filename = f"icon_{points}x{points}" + ("@2x" if scale == 2 else "") + ".png"
            run(["sips", "-z", points * scale, points * scale, work / "icon.png",
                 "--out", icons / filename])
    (stage / ICON).parent.mkdir(parents=True, exist_ok=True)
    run(["iconutil", "--convert", "icns", "--output", stage / ICON, icons])
    shutil.copyfile(work / "background.png", stage / BACKGROUND)


def file_inventory(root):
    inventory = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError("Unexpected link in application")
        if path.is_file():
            inventory[path.relative_to(root).as_posix()] = (sha(path), stat.S_IMODE(path.stat().st_mode))
    return inventory


def finder(mount, configure):
    # argv is passed as data to osascript, never interpolated into AppleScript.
    # Finder touches only this temporary volume; no app is opened or executed.
    common = '''
on run arguments
    set mountPath to item 1 of arguments
    set targetFolder to POSIX file mountPath as alias
    tell application "Finder"
        open targetFolder
        set targetWindow to container window of targetFolder
'''
    if configure:
        body = '''
        set current view of targetWindow to icon view
        set toolbar visible of targetWindow to false
        set statusbar visible of targetWindow to false
        set bounds of targetWindow to {200, 100, 980, 580}
        set viewOptions to icon view options of targetWindow
        set arrangement of viewOptions to not arranged
        set icon size of viewOptions to 112
        set text size of viewOptions to 13
        set background picture of viewOptions to file ".background:background.png" of targetFolder
        set position of item "NextCloud Modern Talk.app" of targetFolder to {204, 235}
        set position of item "Applications" of targetFolder to {576, 235}
        update targetFolder without registering applications
        delay 2
'''
    else:
        body = '''
        set viewOptions to icon view options of targetWindow
        if current view of targetWindow is not icon view then error "DMG icon view missing"
        if icon size of viewOptions is not 112 then error "DMG icon size changed"
        if position of item "NextCloud Modern Talk.app" of targetFolder is not {204, 235} then error "DMG app position changed"
        if position of item "Applications" of targetFolder is not {576, 235} then error "DMG Applications position changed"
        set backgroundFile to get background picture of viewOptions
        set backgroundURL to get URL of backgroundFile
'''
    script = common + body + '''
        close targetWindow
    end tell
''' + ("" if configure else "    return backgroundURL\n") + "end run\n"
    import subprocess
    result = subprocess.run(["osascript", "-", str(mount)], input=script, text=True,
                            capture_output=True, timeout=60)
    if result.returncode:
        details = result.stderr[-2000:]
        position = re.search(r"(?m)^([0-9]{1,8}):([0-9]{1,8}):", details)
        location = ""
        if position:
            start, end = map(int, position.groups())
            if 0 <= start < len(script) and start <= end <= len(script):
                line = script.count("\n", 0, start) + 1
                source = script.splitlines()[line - 1].strip()[:240]
                location = f" at script line {line}, characters {start}:{end}: {source}"
        phase = "configuration" if configure else "read-only inspection"
        raise ValueError(f"Finder disk-image layout failed during {phase}{location}: {details}")
    if not configure:
        # Ask Finder for the actual background file first, without asking its
        # AppleEvent handler to coerce a compound property reference to alias.
        # Bind the returned file URL to this mount, not merely a matching suffix.
        background_url = result.stdout.strip()
        try:
            if len(background_url) > 8192:
                raise ValueError("background URL exceeds its bound")
            parsed = urlsplit(background_url)
            if (parsed.scheme != "file" or parsed.netloc not in ("", "localhost")
                    or parsed.query or parsed.fragment or not parsed.path.startswith("/")):
                raise ValueError("background is not a local file URL")
            actual = Path(unquote(parsed.path, errors="strict"))
            expected = Path(mount) / ".background/background.png"
            if not actual.samefile(expected):
                raise ValueError("background belongs to a different file")
        except (OSError, ValueError) as error:
            raise ValueError("Finder background does not identify this disk image's background: "
                             + repr(background_url[:512])) from error


@contextmanager
def attach(image, mount, writable):
    data = plistlib.loads(run(["hdiutil", "attach", image, "-nobrowse", "-noautoopen",
                              "-readwrite" if writable else "-readonly",
                              "-mountpoint", mount, "-plist"]))
    entities = data.get("system-entities", []) if isinstance(data, dict) else []
    entries = [entry for entry in entities if isinstance(entry, dict)] if isinstance(entities, list) else []
    # Ownership comes only from this attach response. Prefer an explicitly
    # returned whole-disk handle; otherwise retain a returned slice. Grouping
    # avoids detaching the same image once for each partition. Never invent a
    # device path or discover unrelated mounted volumes for cleanup.
    devices = {}
    for entry in entries:
        device = entry.get("dev-entry")
        match = re.fullmatch(r"(/dev/disk[0-9]+)(?:s[0-9]+)*", device) if isinstance(device, str) else None
        if match:
            disk = match.group(1)
            if disk not in devices or device == disk:
                devices[disk] = device
    failure = None
    try:
        volumes = [entry for entry in entries if "mount-point" in entry]
        matches = False
        if len(volumes) == 1:
            reported = volumes[0]["mount-point"]
            device = volumes[0].get("dev-entry")
            if (isinstance(reported, str) and Path(reported).is_absolute()
                    and isinstance(device, str) and re.fullmatch(r"/dev/disk[0-9]+(?:s[0-9]+)*", device)):
                try:
                    # macOS may report /private/var for a requested /var path.
                    # Accept only the same filesystem object, not a name match.
                    matches = Path(mount).samefile(reported)
                except OSError:
                    pass
        if not matches:
            reported = [{key: str(entry[key])[:512] for key in ("dev-entry", "mount-point") if key in entry}
                        for entry in entries[:8]]
            raise ValueError("Disk image did not mount uniquely at its private mount point: "
                             + json.dumps({"requested": str(mount)[:512], "returned": reported}, sort_keys=True))
        yield volumes[0]["dev-entry"]
    except BaseException as error:
        failure = error
        raise
    finally:
        # Attempt detach before outer temporary-directory cleanup, including
        # validation errors raised before yield.
        cleanup_errors = []
        for device in devices.values():
            try:
                run(["hdiutil", "detach", device], 60)
            except Exception as error:
                cleanup_errors.append(f"{device}: {error}")
        if cleanup_errors:
            message = "Disk-image detach failed: " + "; ".join(cleanup_errors)
            if failure is not None:
                failure.add_note(message)
            else:
                raise ValueError(message)


def create(app, output):
    expected = file_inventory(app)
    with tempfile.TemporaryDirectory(prefix="nmt-dmg-") as temporary:
        temporary = Path(temporary)
        contents = temporary / "contents"
        contents.mkdir()
        shutil.copytree(app, contents / APP_NAME)
        (contents / "Applications").symlink_to("/Applications")
        (contents / ".background").mkdir()
        shutil.copyfile(app / "Contents/Resources" / BACKGROUND, contents / ".background/background.png")
        image = temporary / "writable.dmg"
        # HFS+ supports Finder metadata and the familiar drag-to-Applications
        # target; the delivered image is compressed read-only UDZO.
        run(["hdiutil", "create", "-srcfolder", contents, "-volname", VOLUME_NAME,
             "-fs", "HFS+", "-format", "UDRW", image], 180)
        mount = temporary / "mount"
        mount.mkdir()
        with attach(image, mount, True):
            finder(mount, True)
            run(["sync"])
            if not (mount / ".DS_Store").is_file():
                raise ValueError("Finder did not persist the disk-image layout")
        run(["hdiutil", "convert", image, "-format", "UDZO", "-imagekey", "zlib-level=9",
             "-o", output], 180)
        run(["hdiutil", "verify", output], 180)
        with attach(output, mount, False):
            if not os.statvfs(mount).f_flag & os.ST_RDONLY:
                raise ValueError("Delivered disk image is not read-only")
            if not (mount / "Applications").is_symlink() or os.readlink(mount / "Applications") != "/Applications":
                raise ValueError("Missing Applications drag target")
            if file_inventory(mount / APP_NAME) != expected:
                raise ValueError("Disk-image application differs from updater ZIP input")
            if sha(mount / ".background/background.png") != sha(app / "Contents/Resources" / BACKGROUND):
                raise ValueError("Disk-image background differs from source-bound artwork")
            finder(mount, False)
            objects = inspect_bundle(mount / APP_NAME)
            run(["codesign", "--verify", "--deep", "--strict", mount / APP_NAME])
    return {"format": "UDZO", "read_only": True, "applications_target": "/Applications",
            "layout": "Finder icon view, custom background, 112px icons",
            "native_objects": len(objects), "application_executed": False}
