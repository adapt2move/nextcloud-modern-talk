#!/usr/bin/env python3
"""Explicit offline fixtures for the exact locked DS_Store parser.

First download into a private directory with macos_layout.py --download DIR,
then run this file with --tools DIR. No macOS APIs or application code run here.
The normal release fixtures remain offline and exercise the native-tool seam.
"""
import argparse
import base64
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import macos_layout as layout


class PersistedLayoutTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nmt-ds-store-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.path = self.root / ".DS_Store"
        self.options = {**layout.ICON_OPTIONS,
                        "backgroundImageAlias": b"SYNTHETIC alias; never a native bookmark"}
        self.positions = {layout.APP_NAME: (204, 235), "Applications": (576, 235)}
        self.window_options = layout.WINDOW_OPTIONS.copy()
        self.view = b"icnv"

    def write_store(self, extra=0, extra_entries=()):
        from ds_store import DSStore, DSStoreEntry
        from ds_store.store import ILocCodec, PlistCodec
        entries = ([DSStoreEntry(".", b"icvp", PlistCodec, self.options)]
                   if self.options is not None else [])
        entries += [DSStoreEntry(".", b"bwsp", PlistCodec, self.window_options),
                    DSStoreEntry(".", b"icvl", b"type", self.view),
                    DSStoreEntry(".", b"vSrn", b"long", 1)]
        entries += [DSStoreEntry(name, b"Iloc", ILocCodec, position)
                    for name, position in self.positions.items()]
        entries += [DSStoreEntry(f"fixture{n}", b"Iloc", ILocCodec, (0, 0))
                    for n in range(extra)]
        entries += list(extra_entries)
        with DSStore.open(str(self.path), "w+", initial_entries=entries):
            pass

    def test_real_binary_parser_reads_saved_options_and_preserves_exact_alias_and_store(self):
        self.write_store()
        before = self.path.read_bytes()
        reply = layout.read_layout(self.path)
        self.assertEqual(base64.b64decode(reply["background_alias"]), self.options["backgroundImageAlias"])
        self.assertEqual(reply["store_sha256"], hashlib.sha256(before).hexdigest())
        self.assertEqual(reply["entries"], 6)
        self.assertEqual(self.path.read_bytes(), before)

    def test_canonical_writer_roundtrips_real_library_records_and_serialized_alias(self):
        from ds_store import DSStore
        from mac_alias import Alias, VolumeInfo, TargetInfo
        background = self.root / ".background/background.tiff"
        background.parent.mkdir()
        background.write_bytes(b"SYNTHETIC background")
        created = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
        alias = Alias(volume=VolumeInfo("SYNTHETIC", created, b"H+", 0, 0, b"\0\0"),
                      target=TargetInfo(0, "background.tiff", 2, 3, created, b"\0" * 4, b"\0" * 4))
        # Only filesystem metadata discovery is native. Both serialization and
        # the DS_Store writer/reader are the actual pinned libraries.
        with patch.object(Alias, "for_file", return_value=alias) as discover:
            layout.write_layout(self.root)
        discover.assert_called_once_with(str(background))
        reply = layout.read_layout(self.path)
        saved_alias = Alias.from_bytes(base64.b64decode(reply["background_alias"]))
        self.assertEqual(saved_alias.target.filename, "background.tiff")
        self.assertEqual(saved_alias.volume.name, "SYNTHETIC")
        with DSStore.open(str(self.path), "r") as store:
            self.assertEqual(store["."]["bwsp"]["WindowBounds"], "{{120, 120}, {780, 544}}")
            for name in ("ShowToolbar", "ShowSidebar", "ShowStatusBar", "ShowTabView", "ShowPathbar"):
                self.assertIs(store["."]["bwsp"][name], False)
            self.assertEqual(store["."]["icvl"], (b"type", b"icnv"))
            self.assertEqual(store[layout.APP_NAME]["Iloc"], (204, 235))
            self.assertEqual(store["Applications"]["Iloc"], (576, 235))
            self.assertEqual(store["."]["icvp"]["iconSize"], 112.0)
            self.assertEqual(store["."]["icvp"]["textSize"], 13.0)
            self.assertEqual(len(list(store)), 6)

    def test_writer_rejects_readonly_mount_and_symlink_before_discovering_alias(self):
        from mac_alias import Alias
        with patch.object(layout.os, "statvfs", return_value=SimpleNamespace(f_flag=os.ST_RDONLY)), \
                patch.object(Alias, "for_file") as discover:
            with self.assertRaisesRegex(ValueError, "read-only"):
                layout.write_layout(self.root)
            discover.assert_not_called()
        (self.root / ".background").mkdir()
        (self.root / ".background/background.tiff").write_bytes(b"SYNTHETIC background")
        victim = self.root / "victim"
        victim.write_bytes(b"must remain unchanged")
        self.path.symlink_to(victim)
        with patch.object(Alias, "for_file") as discover, self.assertRaisesRegex(ValueError, "Invalid"):
            layout.write_layout(self.root)
        self.assertEqual(victim.read_bytes(), b"must remain unchanged")
        discover.assert_not_called()

    def test_changed_window_bounds_chrome_and_view_are_rejected(self):
        for key, value in (("WindowBounds", "{{0, 0}, {1, 1}}"), ("ShowToolbar", True),
                           ("ShowSidebar", True), ("ShowStatusBar", True)):
            original = self.window_options[key]
            self.window_options[key] = value
            self.write_store()
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "window options"):
                layout.read_layout(self.path)
            self.window_options[key] = original
        self.view = b"Nlsv"
        self.write_store()
        with self.assertRaisesRegex(ValueError, "icon view"):
            layout.read_layout(self.path)

    def test_malformed_required_icon_options_and_positions_are_not_skipped(self):
        from ds_store import DSStoreEntry
        original_options = self.options
        for name, code in ((".", b"icvp"), ("Applications", b"Iloc")):
            self.options = None if code == b"icvp" else original_options
            self.positions = {layout.APP_NAME: (204, 235), "Applications": (576, 235)}
            if code == b"Iloc":
                del self.positions[name]
            self.write_store(extra_entries=[DSStoreEntry(name, code, b"blob", b"bad")])
            with self.subTest(code=code), self.assertRaisesRegex(ValueError, "DS_Store decoding failed"):
                layout.read_layout(self.path)

    def test_persisted_icon_size_text_size_background_kind_and_positions_must_match(self):
        for key, value in (("iconSize", 111), ("textSize", 14), ("backgroundType", 0)):
            original = self.options[key]
            self.options[key] = value
            self.write_store()
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, key):
                layout.read_layout(self.path)
            self.options[key] = original
        for name in self.positions:
            original = self.positions[name]
            self.positions[name] = (1, 2)
            self.write_store()
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "position changed"):
                layout.read_layout(self.path)
            self.positions[name] = original

    def test_missing_and_oversized_or_nonbinary_alias_fail_with_bounded_schema_context(self):
        for alias in (None, "", b""):
            if alias is None:
                self.options.pop("backgroundImageAlias", None)
            else:
                self.options["backgroundImageAlias"] = alias
            self.write_store()
            with self.subTest(alias=type(alias)), self.assertRaisesRegex(ValueError, "background alias") as raised:
                layout.read_layout(self.path)
            self.assertIn("icvp_keys", str(raised.exception))
            self.assertLess(len(str(raised.exception)), 700)
        # Use a smaller policy limit so the fixture remains a normal DS_Store
        # block; the library's writer does not split a single huge plist entry.
        self.options["backgroundImageAlias"] = b"SYNTHETIC oversized alias"
        self.write_store()
        with patch.object(layout, "MAX_ALIAS", 8), self.assertRaisesRegex(ValueError, "background alias"):
            layout.read_layout(self.path)

    def test_missing_options_and_item_position_fail(self):
        original = self.options
        self.options = None
        self.write_store()
        with self.assertRaisesRegex(ValueError, "Missing persisted"):
            layout.read_layout(self.path)
        self.options = original
        del self.positions["Applications"]
        self.write_store()
        with self.assertRaisesRegex(ValueError, "position changed"):
            layout.read_layout(self.path)

    def test_corrupt_allocator_extent_rejected_before_large_allocation(self):
        self.write_store()
        data = bytearray(self.path.read_bytes())
        struct.pack_into(">I", data, 12, 0x7FFFFFFF)
        self.path.write_bytes(data)
        with self.assertRaisesRegex(ValueError, "extent/work limit"):
            layout.read_layout(self.path)

    def test_excessive_records_and_file_size_fail(self):
        self.write_store(extra=layout.MAX_ENTRIES)
        with self.assertRaisesRegex(ValueError, "excessive"):
            layout.read_layout(self.path)
        with self.path.open("wb") as stream:
            stream.truncate(layout.MAX_STORE + 1)
        with self.assertRaisesRegex(ValueError, "bounded regular"):
            layout.read_layout(self.path)

    def test_symlinked_store_is_rejected(self):
        self.write_store()
        link = self.root / "link"
        link.symlink_to(self.path)
        with self.assertRaisesRegex(ValueError, "bounded regular"):
            layout.read_layout(link)

    def test_library_stream_rejects_writes_out_of_extent_and_excessive_work(self):
        for action in (lambda stream: stream.write(b"changed"),
                       lambda stream: stream.read(5),
                       lambda stream: (stream.seek(100), stream.read(1))):
            with self.subTest(action=action), self.assertRaises(ValueError):
                action(layout.StoreInput(b"1234"))
        stream = layout.StoreInput(b"1234")
        stream.remaining = 3
        with self.assertRaisesRegex(ValueError, "work limit"):
            stream.read(4)

    def test_isolated_parser_process_loads_only_the_exact_locked_wheels(self):
        self.write_store()
        reply = json.loads(layout.command([
            sys.executable, "-I", "-S", layout.__file__, "--inspect", self.tools, self.path
        ], 20))
        self.assertEqual(reply["entries"], 6)
        copied = self.root / "tools"
        copied.mkdir()
        for path in layout.checked_wheels(self.tools):
            shutil.copyfile(path, copied / Path(path).name)
        path = copied / layout.locked_tools()[0]["filename"]
        data = bytearray(path.read_bytes())
        data[-1] ^= 1
        path.write_bytes(data)
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            layout.checked_wheels(copied)
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            layout.command([sys.executable, "-I", "-S", layout.__file__,
                            "--inspect", copied, self.path], 20)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools", required=True, type=Path)
    arguments, remaining = parser.parse_known_args()
    PersistedLayoutTests.tools = arguments.tools.resolve()
    sys.path[:0] = layout.checked_wheels(PersistedLayoutTests.tools)
    unittest.main(argv=[sys.argv[0], *remaining])
