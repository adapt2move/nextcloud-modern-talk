#!/usr/bin/env python3
"""Linux structural/failure fixtures; no native executable is ever run."""
import copy
import base64
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import plistlib
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

import macos_runtime as runtime
import macos_dmg as dmg
import macos_layout as layout


def command(kind, text):
    offset = 24 if kind in (0xC, 0xD) else 12
    data = text.encode() + b"\0"
    size = (offset + len(data) + 7) // 8 * 8
    return struct.pack("<III", kind, size, offset) + bytes(offset - 12) + data + bytes(size - offset - len(data))


def native(path, kind=6, dependencies=("/usr/lib/libSystem.B.dylib",), rpaths=()):
    commands = ([command(0xD, "@rpath/" + path.name)] if kind == 6 else [])
    commands += [command(0xC, name) for name in dependencies]
    commands += [command(0x8000001C, name) for name in rpaths]
    payload = b"".join(commands)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(struct.pack("<8I", 0xFEEDFACF, 0x0100000C, 0, kind, len(commands), len(payload), 0, 0)
                     + payload + b"SYNTHETIC NONEXECUTABLE TEST DATA")
    return path


class MacRuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nmt-native-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_all_admitted_plugins_have_locked_corresponding_sources(self):
        lock = json.loads((Path(__file__).with_name("macos-sources.json")).read_text())
        declared = [name for source in lock["sources"] for name in source["runtime_files"]]
        self.assertEqual(len(declared), len(set(declared)))
        required = {f"lib/gstreamer-1.0/libgst{name}.dylib" for name in runtime.PLUGINS}
        self.assertTrue(required <= set(declared), sorted(required - set(declared)))
        # The attachment player selects local native/LGPL codec branches. An
        # SDK's incidental GPL decoder or network/autoplug plugin is not consent
        # to redistribute or use it in the account-bound content owner.
        self.assertFalse({"libav", "x264", "x265", "soup", "playback"} & runtime.PLUGINS.keys())

    def test_arm64_universal_selection_and_load_command_bounds(self):
        thin = native(self.root / "libfixture.dylib").read_bytes()
        fat = self.root / "fat.dylib"
        fat.write_bytes(struct.pack(">II5I", 0xCAFEBABE, 1, 0x0100000C, 0, 32, len(thin), 0) + bytes(4) + thin)
        self.assertEqual(runtime.macho(fat)["dependencies"], ["/usr/lib/libSystem.B.dylib"])
        for changed in (b"\xca\xfe\xba\xbe" + struct.pack(">I", 100) + bytes(24),
                        thin[:20], thin[:20] + struct.pack("<I", 0x7FFFFFFF) + thin[24:]):
            fat.write_bytes(changed)
            with self.assertRaises(ValueError):
                runtime.macho(fat)

    def test_homebrew_absolute_unknown_and_traversing_inputs_are_rejected(self):
        for name in ("/opt/homebrew/lib/libglib.dylib", "@rpath/../libglib.dylib",
                     "@loader_path/libglib.dylib", "/tmp/libglib.dylib",
                     "/Library/Frameworks/Other.framework/libglib.dylib"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                runtime.library_relative(name)
        self.assertFalse(runtime.system_library("/usr/lib/../../tmp/libfake.dylib"))

    def test_dependency_cycles_terminate_but_missing_transitive_libraries_fail(self):
        sdk = self.root / "sdk"
        names = ["lib/liba.dylib", "lib/libb.dylib"]
        for index, name in enumerate(names):
            native(sdk / name, dependencies=("@rpath/" + Path(names[1 - index]).name,))
        native(sdk / runtime.SCANNER, 2)
        for plugin in runtime.PLUGINS:
            native(sdk / f"lib/gstreamer-1.0/libgst{plugin}.dylib")
        executable = native(self.root / "talk-desktop", 2, ("@rpath/liba.dylib",))
        closure = runtime.closure(runtime.expanded_finder([sdk]), executable)
        self.assertTrue(set(names) <= closure.keys())
        (sdk / names[1]).unlink()
        with self.assertRaisesRegex(ValueError, "Missing"):
            runtime.closure(runtime.expanded_finder([sdk]), executable)

    def test_pkg_alias_escape_conflicts_and_closure_budget_fail(self):
        a, b = self.root / "a", self.root / "b"
        library = native(a / "lib/liba.dylib")
        native(b / "lib/liba.dylib", dependencies=("@rpath/libb.dylib",))
        with self.assertRaisesRegex(ValueError, "conflicting"):
            runtime.expanded_finder([a, b])("lib/liba.dylib")
        library.unlink()
        library.symlink_to(b / "lib/liba.dylib")
        with self.assertRaisesRegex(ValueError, "escapes"):
            runtime.expanded_finder([a])("lib/liba.dylib")
        executable = native(self.root / "talk-desktop", 2)
        native(b / runtime.SCANNER, 2)
        native(b / "lib/gstreamer-1.0/libgstapp.dylib")
        with (patch.object(runtime, "MAX_FILES", 1), patch.object(runtime, "PLUGINS", {"app": ["appsrc"]}),
              self.assertRaisesRegex(ValueError, "closure exceeds")):
            runtime.closure(runtime.expanded_finder([b]), executable)

    def app_fixture(self):
        app = self.root / "NextCloud Modern Talk.app"
        native(app / "Contents/Frameworks/libgstreamer-1.0.0.dylib")
        core = "@loader_path/../Frameworks/libgstreamer-1.0.0.dylib"
        native(app / "Contents/MacOS/talk-desktop", 2, (core,))
        native(app / "Contents/Helpers/gst-plugin-scanner", 2, (core,))
        for plugin in runtime.PLUGINS:
            native(app / f"Contents/PlugIns/libgst{plugin}.dylib", 6, (core,))
        return app

    def test_complete_layout_inspection_is_location_independent(self):
        app = self.app_fixture()
        before = runtime.inspect_bundle(app)
        parent = self.root / "Moved directory with Ünicode"
        parent.mkdir()
        moved = app.rename(parent / app.name)
        self.assertEqual(runtime.inspect_bundle(moved), before)

    def test_inspection_rejects_unresolved_edges_and_scanner_relative_path_error(self):
        app = self.app_fixture()
        scanner = app / "Contents/Helpers/gst-plugin-scanner"
        native(scanner, 2, ("@loader_path/libgstreamer-1.0.0.dylib",))
        with self.assertRaisesRegex(ValueError, "Unresolved"):
            runtime.inspect_bundle(app)
        native(scanner, 2, ("@loader_path/../../../outside.dylib",))
        with self.assertRaisesRegex(ValueError, "escaping"):
            runtime.inspect_bundle(app)

    def test_inspection_rejects_external_runpaths_unrelocated_loads_and_missing_plugins(self):
        app = self.app_fixture()
        binary = app / "Contents/MacOS/talk-desktop"
        native(binary, 2, rpaths=(runtime.INSTALLED + "lib",))
        with self.assertRaisesRegex(ValueError, "search path"):
            runtime.inspect_bundle(app)
        native(binary, 2, ("@rpath/libgstreamer-1.0.0.dylib",))
        with self.assertRaisesRegex(ValueError, "Non-relocated"):
            runtime.inspect_bundle(app)
        native(binary, 2)
        (app / "Contents/PlugIns/libgstopus.dylib").unlink()
        with self.assertRaisesRegex(ValueError, "missing"):
            runtime.inspect_bundle(app)

    def test_inspection_rejects_code_resources_and_runtime_symlinks(self):
        app = self.app_fixture()
        extra = native(app / "Contents/Resources/assets/hidden.dylib")
        with self.assertRaisesRegex(ValueError, "code location"):
            runtime.inspect_bundle(app)
        extra.unlink()
        extra.symlink_to(app / "Contents/Frameworks/libgstreamer-1.0.0.dylib")
        with self.assertRaisesRegex(ValueError, "Symlink"):
            runtime.inspect_bundle(app)

    def test_relocation_uses_each_loader_and_removes_all_old_runpaths(self):
        path = native(self.root / "libgstapp.dylib", dependencies=("@rpath/libgstapp-1.0.0.dylib",),
                      rpaths=("/opt/homebrew/lib", runtime.INSTALLED + "lib"))
        commands = []
        with patch.object(runtime, "run", side_effect=lambda argv: commands.append(argv)):
            runtime.relocate(path, runtime.macho(path), Path("Contents/PlugIns/libgstapp.dylib"),
                             {"lib/libgstapp-1.0.0.dylib": Path("Contents/Frameworks/libgstapp-1.0.0.dylib")})
        self.assertIn("@loader_path/../Frameworks/libgstapp-1.0.0.dylib", commands[0])
        self.assertEqual(commands[0].count("-delete_rpath"), 2)
        self.assertEqual(commands[-1][:3], ["codesign", "--verify", "--strict"])

    def test_source_coverage_and_license_path_escape_fail_closed(self):
        lock = {"schema": 1, "runtime_version": runtime.VERSION, "sources": [
            {"name": "fixture", "filename": "fixture.tar.gz", "url": "https://invalid.test/source",
             "sha256": "0" * 64, "runtime_files": ["lib/libfixture.dylib"]}]}
        root, stage = self.root / "repo", self.root / "stage"
        path = root / "tools/release/macos-sources.json"
        path.parent.mkdir(parents=True)
        path.write_text(json.dumps(lock))
        with self.assertRaisesRegex(
                ValueError,
                r"source/license coverage: uncovered=\['lib/libother.dylib'\], "
                r"unused=\['lib/libfixture.dylib'\]"):
            runtime.collect_sources(root, stage, {"lib/libother.dylib": None})
        source = self.root / "fixture.tar.gz"
        with tarfile.open(source, "w:gz") as archive:
            info = tarfile.TarInfo("../LICENSE")
            info.size = 7
            archive.addfile(info, io.BytesIO(b"fixture"))
        with patch.object(runtime, "download", side_effect=lambda url, digest, dest, limit: shutil.copyfile(source, dest)):
            with self.assertRaisesRegex(ValueError, "Unsafe source"):
                runtime.collect_sources(root, stage, {"lib/libfixture.dylib": None})

    def test_exact_runtime_mapping_forbids_extra_namespace_and_traversal(self):
        for name in ("runtime/gstreamer/lib/../surprise.dylib", "runtime/gstreamer/bin/gst-launch",
                     "runtime/gstreamer/lib/libbad.dylib/extra", "runtime/gstreamer/share/anything",
                     "runtime/gstreamer/lib/gstreamer-1.0/libgstunknown.dylib"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                runtime.bundle_path(name)

    def test_main_identity_uses_private_copy_and_keeps_the_signed_input_unchanged(self):
        signed = self.root / "talk-desktop"
        original = b"SYNTHETIC code" + b"SYNTHETIC signature"
        signed.write_bytes(original)
        copies = []

        def remove(argv, timeout):
            self.assertEqual(argv[:2], ["/usr/bin/codesign", "--remove-signature"])
            self.assertEqual(timeout, 30)
            copied = Path(argv[2])
            copies.append(copied)
            self.assertNotEqual(copied, signed)
            self.assertEqual(copied.stat().st_mode & 0o777, 0o600)
            self.assertEqual(copied.parent.stat().st_mode & 0o777, 0o700)
            self.assertEqual(copied.read_bytes(), original)
            copied.write_bytes(b"SYNTHETIC code")

        with patch.object(runtime, "run", side_effect=remove):
            actual = runtime.unsigned_executable_sha256(signed)
        self.assertEqual(actual, hashlib.sha256(b"SYNTHETIC code").hexdigest())
        self.assertEqual(signed.read_bytes(), original)
        self.assertFalse(copies[0].parent.exists())

    def test_main_identity_failure_removes_copy_and_preserves_input(self):
        signed = self.root / "talk-desktop"
        signed.write_bytes(b"SYNTHETIC code")
        copies = []

        def timeout(argv, timeout):
            copies.append(Path(argv[2]))
            raise subprocess.TimeoutExpired(argv, timeout)

        with patch.object(runtime, "run", side_effect=timeout), self.assertRaises(subprocess.TimeoutExpired):
            runtime.unsigned_executable_sha256(signed)
        self.assertEqual(signed.read_bytes(), b"SYNTHETIC code")
        self.assertFalse(copies[0].parent.exists())
        with patch.object(runtime, "MAX_BYTES", 1), self.assertRaises(ValueError):
            runtime.unsigned_executable_sha256(signed)
        link = self.root / "linked"
        link.symlink_to(signed)
        with self.assertRaises(ValueError):
            runtime.unsigned_executable_sha256(link)

    def test_dmg_attach_accepts_a_physical_path_alias_and_detaches_its_own_disk(self):
        physical = self.root / "private"
        physical.mkdir()
        (physical / "mount").mkdir()
        alias = self.root / "var"
        alias.symlink_to(physical, target_is_directory=True)
        mount = alias / "mount"
        response = plistlib.dumps({"system-entities": [
            {"dev-entry": "/dev/disk27"},
            {"dev-entry": "/dev/disk27s1", "mount-point": str(physical / "mount")},
        ]})
        for writable in (True, False):
            with self.subTest(writable=writable), patch.object(dmg, "run", side_effect=[response, b""]) as run:
                with dmg.attach(self.root / "fixture.dmg", mount, writable) as device:
                    self.assertEqual(device, "/dev/disk27s1")
                    self.assertEqual(run.call_count, 1)
                self.assertIn("-readwrite" if writable else "-readonly", run.call_args_list[0].args[0])
                self.assertEqual(run.call_args_list[1].args, (["hdiutil", "detach", "/dev/disk27"], 60))

    def test_dmg_wrong_missing_and_multiple_mounts_fail_after_owned_cleanup(self):
        mount, wrong = self.root / "mount", self.root / "wrong"
        mount.mkdir()
        wrong.mkdir()
        cases = [
            [{"dev-entry": "/dev/disk27s1", "mount-point": str(wrong)}],
            [{"dev-entry": "/dev/disk27s1", "mount-point": str(self.root / "missing")}],
            [{"dev-entry": "/dev/disk27s1", "mount-point": str(mount)},
             {"dev-entry": "/dev/disk27s2", "mount-point": str(wrong)}],
        ]
        for volumes in cases:
            response = plistlib.dumps({"system-entities": [{"dev-entry": "/dev/disk27"}, *volumes]})
            with self.subTest(volumes=volumes), patch.object(dmg, "run", side_effect=[response, b""]) as run:
                with self.assertRaisesRegex(ValueError, "did not mount uniquely") as raised:
                    with dmg.attach(self.root / "fixture.dmg", mount, True):
                        self.fail("Invalid mount must never reach Finder or image validation")
                self.assertIn(str(mount), str(raised.exception))
                self.assertIn(volumes[0]["mount-point"], str(raised.exception))
                self.assertEqual(run.call_count, 2)
                self.assertEqual(run.call_args.args, (["hdiutil", "detach", "/dev/disk27"], 60))

    def test_dmg_work_failure_detaches_the_exact_returned_slice_without_discovery(self):
        mount = self.root / "mount"
        mount.mkdir()
        response = plistlib.dumps({"system-entities": [
            {"dev-entry": "/dev/disk27s1", "mount-point": str(mount)},
        ]})
        with patch.object(dmg, "run", side_effect=[response, b""]) as run:
            with self.assertRaisesRegex(ValueError, "SYNTHETIC Finder failure"):
                with dmg.attach(self.root / "fixture.dmg", mount, True):
                    raise ValueError("SYNTHETIC Finder failure")
            self.assertEqual(run.call_count, 2)
            self.assertEqual(run.call_args.args, (["hdiutil", "detach", "/dev/disk27s1"], 60))

    def test_dmg_attach_failure_does_not_guess_a_cleanup_device(self):
        with patch.object(dmg, "run", side_effect=ValueError("SYNTHETIC attach failure")) as run:
            with self.assertRaisesRegex(ValueError, "SYNTHETIC attach failure"):
                with dmg.attach(self.root / "fixture.dmg", self.root / "mount", True):
                    self.fail("Failed attachment must not be used")
            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.args[0][:2], ["hdiutil", "attach"])

    def test_dmg_detach_failure_is_reported_without_losing_mount_validation_details(self):
        mount = self.root / "mount"
        mount.mkdir()
        response = plistlib.dumps({"system-entities": [
            {"dev-entry": "/dev/disk27", "mount-point": str(self.root / "wrong")},
        ]})
        with patch.object(dmg, "run", side_effect=[response, ValueError("SYNTHETIC detach failure")]):
            with self.assertRaisesRegex(ValueError, "did not mount uniquely") as raised:
                with dmg.attach(self.root / "fixture.dmg", mount, True):
                    self.fail("Invalid mount must not be used")
            self.assertIn("/dev/disk27: SYNTHETIC detach failure", raised.exception.__notes__[0])

    def layout_response(self):
        return json.dumps({"background_alias": base64.b64encode(b"SYNTHETIC alias").decode(),
                           "store_sha256": "1" * 64, "entries": 3}).encode()

    def test_persisted_background_resolution_binds_the_alias_to_the_current_mount(self):
        mount = self.root / "physical mount Ü"
        background = mount / ".background/background.tiff"
        background.parent.mkdir(parents=True)
        background.write_bytes(b"SYNTHETIC background")
        alias = self.root / "mount alias"
        alias.symlink_to(mount, target_is_directory=True)
        image = {"format": "TIFF", "points": [780, 480],
                 "representations": [{"pixels": [780, 480], "dpi": [72, 72]},
                                     {"pixels": [1560, 960], "dpi": [144, 144]}]}
        for url in (background.as_uri(), background.as_uri().replace("file:///", "file://localhost/", 1)):
            reply = json.dumps({"url": url, "stale": True, "background_image": image}).encode()
            with patch.object(layout, "command", side_effect=[self.layout_response(), reply]) as run:
                report = layout.inspect(alias, (self.root / "wheels", self.root / "resolver"))
            self.assertTrue(report["alias_stale"])  # Remounting can stale a correctly resolved bookmark.
            self.assertEqual(report["background_image"], image)
            self.assertEqual(run.call_args_list[0].args[0][1:3], ["-I", "-S"])
            self.assertEqual(run.call_args_list[0].args[1], 20)
            self.assertEqual(run.call_args_list[1].args, (
                [self.root / "resolver", alias], 30, b"SYNTHETIC alias", 16384))
        with patch("subprocess.run", return_value=subprocess.CompletedProcess([], 0, "", "")) as run:
            dmg.finder(alias)
            script = run.call_args.kwargs["input"]
            self.assertNotIn("get background picture", script)
            for assertion in ("DMG icon view missing", "DMG icon size changed",
                              "DMG app position changed", "DMG Applications position changed"):
                self.assertIn(assertion, script)
            self.assertEqual(run.call_args.args[0], ["osascript", "-", str(alias)])

    def test_persisted_background_existence_or_a_matching_suffix_is_not_sufficient(self):
        mount = self.root / "mount"
        background = mount / ".background/background.tiff"
        background.parent.mkdir(parents=True)
        background.write_bytes(b"SYNTHETIC background")
        wrong = self.root / "another mount/.background/background.tiff"
        wrong.parent.mkdir(parents=True)
        wrong.write_bytes(background.read_bytes())
        for url in (wrong.as_uri(), "", "missing value", "https://invalid.test/background.tiff",
                    "file://remote.invalid" + str(background), background.as_uri() + "?ignored=true",
                    background.as_uri() + "#fragment", "file:///" + "a" * 8192):
            reply = json.dumps({"url": url, "stale": False}).encode()
            with self.subTest(url=url[:80]), patch.object(
                layout, "command", side_effect=[self.layout_response(), reply]):
                with self.assertRaisesRegex(ValueError, "does not identify this disk image") as raised:
                    layout.inspect(mount, (self.root / "wheels", self.root / "resolver"))
                self.assertLess(len(str(raised.exception)), 700)

    def test_persisted_alias_failure_stops_inspection_and_reports_bounded_native_details(self):
        for failure in (ValueError("SYNTHETIC rejected alias"),
                        subprocess.TimeoutExpired("SYNTHETIC resolver", 30)):
            with self.subTest(failure=failure), patch.object(
                layout, "command", side_effect=[self.layout_response(), failure]) as run:
                with self.assertRaises(type(failure)):
                    layout.inspect(self.root, (self.root / "wheels", self.root / "resolver"))
                self.assertEqual(run.call_count, 2)
        with patch("subprocess.run", return_value=subprocess.CompletedProcess(
                [], 1, b"", b"SYNTHETIC " * 1000)):
            with self.assertRaisesRegex(ValueError, "layout tool failed") as raised:
                layout.command(["SYNTHETIC resolver"], 30, b"alias")
            self.assertLess(len(str(raised.exception)), 2100)
        with patch("subprocess.run", return_value=subprocess.CompletedProcess([], 0, b"12345", b"")):
            with self.assertRaisesRegex(ValueError, "output limit"):
                layout.command(["SYNTHETIC parser"], 20, limit=4)

    def test_layout_child_timeout_kills_and_reaps_the_child(self):
        pidfile = self.root / "child.pid"
        script = ("import os, pathlib, sys, time; "
                  "pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(30)")
        with self.assertRaises(subprocess.TimeoutExpired):
            layout.command([sys.executable, "-I", "-S", "-c", script, pidfile], 0.5)
        pid = int(pidfile.read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_dmg_delivery_requires_persisted_inspection_before_final_native_verification(self):
        app = self.root / dmg.APP_NAME
        background = app / "Contents/Resources" / dmg.BACKGROUND
        background.parent.mkdir(parents=True)
        background.write_bytes(b"SYNTHETIC background")
        events = []

        @contextlib.contextmanager
        def attached(image, mount, writable):
            if writable:
                shutil.copytree(app, mount / dmg.APP_NAME)
                (mount / "Applications").symlink_to("/Applications")
                (mount / ".background").mkdir()
                shutil.copyfile(background, mount / ".background/background.tiff")
                (mount / ".DS_Store").write_bytes(b"SYNTHETIC parser input")
            try:
                yield "/dev/SYNTHETIC"
            finally:
                events.append(("detached", writable))

        with patch.object(layout, "prepare", return_value=("wheels", "resolver")), \
                patch.object(layout, "write") as write, \
                patch.object(dmg, "attach", side_effect=attached), \
                patch.object(dmg, "finder"), patch.object(dmg, "run") as run, \
                patch.object(dmg.os, "statvfs", return_value=SimpleNamespace(f_flag=os.ST_RDONLY)), \
                patch.object(dmg, "inspect_bundle", return_value=[]) as native_check:
            with patch.object(layout, "inspect", side_effect=ValueError("SYNTHETIC wrong alias")):
                with self.assertRaisesRegex(ValueError, "wrong alias"):
                    dmg.create(app, self.root / "fixture.dmg")
            self.assertEqual(events, [("detached", True), ("detached", False)])
            write.assert_called_once()
            native_check.assert_not_called()
            self.assertFalse(any(call.args[0][0] == "codesign" for call in run.call_args_list))
            with patch.object(layout, "inspect", return_value={"SYNTHETIC": "verified"}) as inspect:
                report = dmg.create(app, self.root / "fixture.dmg")
            self.assertEqual(report["persisted_layout"], {"SYNTHETIC": "verified"})
            inspect.assert_called_once()
            self.assertEqual(run.call_args.args[0][:4], ["codesign", "--verify", "--deep", "--strict"])

    def test_finder_native_error_identifies_the_actual_script_line_with_bounded_output(self):
        def failed(argv, **kwargs):
            script = kwargs["input"]
            start = script.index("current view")
            return subprocess.CompletedProcess(
                argv, 1, "", f"{start}:{start + 3}: execution error: SYNTHETIC AppleEvent failure (-10000)\n"
            )

        with patch("subprocess.run", side_effect=failed):
            with self.assertRaisesRegex(ValueError, "read-only inspection at script line") as raised:
                dmg.finder(self.root / "mount")
            self.assertIn("DMG icon view missing", str(raised.exception))
            self.assertIn("(-10000)", str(raised.exception))
        with patch("subprocess.run", return_value=subprocess.CompletedProcess(
            [], 1, "", "99999999:99999999: " + "SYNTHETIC " * 1000
        )):
            with self.assertRaises(ValueError) as raised:
                dmg.finder(self.root / "mount")
            self.assertLess(len(str(raised.exception)), 2300)


class MacPackageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nmt-mac-package-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        # Import lazily to avoid a circular import when release/tests.py also
        # discovers these classes as part of the existing Linux test command.
        from tests import fixture
        self.stage = self.root / "input"
        self.spec = fixture(self.stage, "aarch64-apple-darwin")
        self.records = {}
        names = [runtime.SCANNER, "lib/libgstreamer-1.0.0.dylib",
                 *("lib/gstreamer-1.0/libgst" + p + ".dylib" for p in runtime.PLUGINS)]
        for name in names:
            path = native(self.stage / runtime.PREFIX / name, 2 if name == runtime.SCANNER else 6)
            digest = runtime.sha(path)
            self.spec["files"][runtime.PREFIX + name] = digest
            self.records[name] = {"sha256": digest, "source_sha256": "1" * 64,
                                  "bundle_path": runtime.bundle_path(runtime.PREFIX + name), **runtime.macho(path)}
        self.manifest = {"schema": 1, "version": runtime.VERSION,
                         "runtime_archive": {"url": runtime.RUNTIME_URL, "sha256": runtime.RUNTIME_SHA256},
                         "plugins": runtime.PLUGINS, "files": self.records,
                         "source_distribution": {"sources": [{"filename": "fixture.tar.xz", "sha256": "2" * 64,
                                                               "notices": ["licenses/project/LICENSE"],
                                                               "runtime_files": names}]}}
        self.spec["files"]["licenses/gstreamer/sources/fixture.tar.xz"] = "2" * 64
        self.bind()

    def bind(self):
        path = self.stage / runtime.MANIFEST
        path.write_text(json.dumps(self.manifest))
        self.spec["files"][runtime.MANIFEST] = runtime.sha(path)
        self.spec["build"]["bundled_runtime"] = {
            "name": "GStreamer", "version": runtime.VERSION, "manifest": runtime.MANIFEST,
            "manifest_sha256": runtime.sha(path)}

    def test_manifest_requires_complete_plugins_scanner_source_and_identity(self):
        runtime.validate_inputs(self.stage, self.spec)
        original = copy.deepcopy(self.spec)
        for name in ("lib/gstreamer-1.0/libgstopus.dylib", runtime.SCANNER):
            del self.spec["files"][runtime.PREFIX + name]
            with self.assertRaises(ValueError):
                runtime.validate_inputs(self.stage, self.spec)
            self.spec = copy.deepcopy(original)
        del self.spec["files"]["licenses/gstreamer/sources/fixture.tar.xz"]
        with self.assertRaisesRegex(ValueError, "source missing"):
            runtime.validate_inputs(self.stage, self.spec)
        self.spec = original
        self.manifest["runtime_archive"]["sha256"] = "f" * 64
        self.bind()
        with self.assertRaisesRegex(ValueError, "required closure"):
            runtime.validate_inputs(self.stage, self.spec)

    def test_manifest_cannot_remap_a_hash_bound_native_file(self):
        self.records[runtime.SCANNER]["bundle_path"] = "Contents/Resources/scanner"
        self.bind()
        with self.assertRaisesRegex(ValueError, "bytes/layout"):
            runtime.validate_inputs(self.stage, self.spec)

    def test_runtime_cannot_leak_into_linux_package(self):
        self.spec["target"] = "x86_64-unknown-linux-gnu"
        with self.assertRaisesRegex(ValueError, "different target"):
            runtime.validate_inputs(self.stage, self.spec)

    def complete_source(self):
        source = self.stage / "licenses/gstreamer/sources/fixture.tar.xz"
        source.parent.mkdir(parents=True)
        source.write_bytes(b"SYNTHETIC source fixture, not a usable source archive")
        digest = runtime.sha(source)
        self.spec["files"][source.relative_to(self.stage).as_posix()] = digest
        self.manifest["source_distribution"]["sources"][0]["sha256"] = digest
        self.bind()

    def test_zip_carries_code_in_standard_locations_and_resource_provenance(self):
        from package import assemble
        from tests import save
        self.complete_source()
        save(self.stage, self.spec)
        output = self.root / "output"
        assemble(self.stage, output, allow_fixture=True)
        self.assertEqual(len(list(output.glob("*.dmg"))), 0)  # Native DMG creation is not simulated.
        with zipfile.ZipFile(next(output.glob("*.zip"))) as archive:
            prefix = "NextCloud Modern Talk.app/"
            for relative, record in self.records.items():
                physical = prefix + record["bundle_path"]
                self.assertIn(physical, archive.namelist())
                self.assertEqual((archive.getinfo(physical).external_attr >> 16) & 0o777, 0o755)
            self.assertIn(prefix + "Contents/Resources/provenance.json", archive.namelist())
            self.assertNotIn(prefix + "Contents/_CodeSignature/PackageProvenance.json", archive.namelist())
            self.assertNotIn(b"Required before launch", archive.read(prefix + "Contents/Resources/INSTALL.txt"))

    def prepare_signing_fixture(self):
        from tests import save
        import macos_dmg
        self.complete_source()
        self.spec["kind"] = "development"
        for name in (macos_dmg.ICON, macos_dmg.BACKGROUND):
            path = self.stage / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"SYNTHETIC artwork")
            self.spec["files"][name] = runtime.sha(path)
        main = native(self.stage / "bin/talk-desktop", 2)
        self.spec["files"]["bin/talk-desktop"] = runtime.sha(main)
        save(self.stage, self.spec)

    def test_signing_seals_provenance_first_and_never_mutates_the_signed_tree(self):
        from package import assemble
        import macos_dmg
        self.prepare_signing_fixture()
        unsigned = (self.stage / "bin/talk-desktop").read_bytes()
        signature = b"SYNTHETIC native signature"
        snapshots = []

        def normalized(path):
            return hashlib.sha256(Path(path).read_bytes().removesuffix(signature)).hexdigest()

        def command(argv):
            if argv[0] != "codesign":
                return b""
            app = Path(argv[-1])
            provenance = json.loads((app / "Contents/Resources/provenance.json").read_bytes())
            self.assertEqual(provenance["build"]["macos_main_identity"], runtime.MAIN_IDENTITY)
            self.assertEqual(provenance["files"]["bin/talk-desktop"], hashlib.sha256(unsigned).hexdigest())
            if argv[1] == "--force":
                self.assertEqual(sorted(p.name for p in app.iterdir()), ["Contents"])
                (app / "Contents/MacOS/talk-desktop").write_bytes(unsigned + signature)
                seal = app / "Contents/_CodeSignature/CodeResources"
                seal.parent.mkdir()
                seal.write_bytes(b"SYNTHETIC native resource seal")
                self.assertEqual(seal.stat().st_mode & 0o777, 0o644)
                snapshots.append(macos_dmg.file_inventory(app))
            else:
                self.assertEqual(argv[1:4], ["--verify", "--deep", "--strict"])
                self.assertEqual(macos_dmg.file_inventory(app), snapshots[0])
            return b""

        def dmg(app, output):
            self.assertEqual(macos_dmg.file_inventory(app), snapshots[0])
            output.write_bytes(b"SYNTHETIC DMG")
            return {"application_executed": False}

        old_umask = os.umask(0o077)
        try:
            with (patch.object(runtime, "run", side_effect=command),
                  patch.object(runtime, "unsigned_executable_sha256", side_effect=normalized),
                  patch.object(macos_dmg, "create", side_effect=dmg),
                  contextlib.redirect_stdout(io.StringIO())):
                output = self.root / "output"
                report = assemble(self.stage, output)
                self.assertEqual(os.umask(0o077), 0o077)
        finally:
            os.umask(old_umask)
        self.assertEqual(report["main_executable"]["sha256"], hashlib.sha256(unsigned + signature).hexdigest())
        with zipfile.ZipFile(next(output.glob("*.zip"))) as archive:
            for name, (digest, _) in snapshots[0].items():
                self.assertEqual(hashlib.sha256(archive.read("NextCloud Modern Talk.app/" + name)).hexdigest(), digest)

    def test_changed_canonical_identity_aborts_before_disk_image_creation(self):
        from package import assemble
        import macos_dmg
        self.prepare_signing_fixture()
        with (patch.object(runtime, "run"),
              patch.object(runtime, "unsigned_executable_sha256", side_effect=["a" * 64, "b" * 64]),
              patch.object(macos_dmg, "create") as create,
              contextlib.redirect_stdout(io.StringIO()),
              self.assertRaisesRegex(ValueError, "signature-independent main")):
            assemble(self.stage, self.root / "output")
        create.assert_not_called()

    def test_native_verification_failure_aborts_before_disk_image_creation(self):
        from package import assemble
        import macos_dmg
        self.prepare_signing_fixture()

        def command(argv):
            if argv[:4] == ["codesign", "--verify", "--deep", "--strict"]:
                raise ValueError("SYNTHETIC native verification rejection")
            return b""

        with (patch.object(runtime, "run", side_effect=command),
              patch.object(runtime, "unsigned_executable_sha256", return_value="a" * 64),
              patch.object(macos_dmg, "create") as create,
              self.assertRaisesRegex(ValueError, "native verification rejection")):
            assemble(self.stage, self.root / "output")
        create.assert_not_called()


if __name__ == "__main__":
    unittest.main()
