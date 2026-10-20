#!/usr/bin/env python3
"""Delivery-only fixtures. No Cargo, application, native store, browser or service."""
import hashlib
import contextlib
import io
import json
import os
from pathlib import Path
import plistlib
import shutil
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from build import dependency_notices, macos_linkage, notice_supplement
from package import REQUIRED_NOTICES, assemble, sha, validate


def fixture(path, target="x86_64-unknown-linux-gnu"):
    path.mkdir()
    header = bytearray(64)
    if target.startswith("x86_64"):
        header[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", header, 18, 62)
    else:
        header[:4] = b"\xcf\xfa\xed\xfe"
        struct.pack_into("<I", header, 4, 0x0100000C)
    files = {"bin/talk-desktop": bytes(header), **{n: b"SYNTHETIC packaging test notice, not a project license" for n in REQUIRED_NOTICES}}
    files.update({"assets/fonts/Manrope-" + weight + ".ttf": b"SYNTHETIC font fixture" for weight in ("Regular", "SemiBold", "Bold")})
    files["source-manifest.json"] = json.dumps({"Cargo.lock": "1" * 64, "rust-toolchain.toml": "2" * 64}).encode()
    files["dependencies.json"] = json.dumps({"target": target, "lock_sha256": "1" * 64, "packages": [{"name": "synthetic-fixture", "license": "fixture-only", "features": [], "notices": [REQUIRED_NOTICES[0]]}]}).encode()
    for name, body in files.items():
        (path / name).parent.mkdir(parents=True, exist_ok=True)
        (path / name).write_bytes(body)
    spec = {"schema": 1, "kind": "packaging-test-fixture", "target": target, "version": "0.0.0-dev.fixture",
            "source": {"commit": "a" * 40, "tree": "b" * 40, "epoch": 1700000000, "manifest_sha256": sha(path / "source-manifest.json"), "lock_sha256": "1" * 64, "toolchain_sha256": "2" * 64},
            "build": {"package": "talk-desktop", "profile": "release", "features": [], "debian_depends": "libc6 (>= 2.39)"},
            "media": {"enabled": False}, "files": {n: sha(path / n) for n in files}}
    save(path, spec)
    return spec


def save(path, spec):
    (path / "input.json").write_text(json.dumps(spec, indent=2) + "\n")


class MacLinkageFixtures(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="nmt-mac-link-test-")
        self.addCleanup(self.temp.cleanup)
        self.runtime = Path(self.temp.name) / "GStreamer.framework/lib"
        self.runtime.mkdir(parents=True)
        self.library = self.runtime / "libgstapp-1.0.0.dylib"
        self.library.write_bytes(b"Synthetic library, not a Mach-O executable")
        self.linked = ["\t@rpath/libgstapp-1.0.0.dylib (compatibility version 2805.0.0, current version 2805.0.0)",
                       "\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1356.0.0)"]
        self.commands = f"Load command 1\n          cmd LC_RPATH\n      cmdsize 112\n         path {self.runtime} (offset 12)\n"

    def test_runtime_library_requires_both_installed_file_and_executable_runpath(self):
        result = macos_linkage(self.linked, self.commands, self.runtime)
        self.assertEqual(result["rpaths"], [str(self.runtime)])
        with self.assertRaisesRegex(ValueError, "unresolved"):
            macos_linkage(self.linked, "", self.runtime)
        self.library.unlink()
        with self.assertRaisesRegex(ValueError, "unresolved"):
            macos_linkage(self.linked, self.commands, self.runtime)

    def test_other_installations_and_path_escapes_are_rejected(self):
        for name in ("/opt/homebrew/lib/libgstapp-1.0.0.dylib", "@rpath/../libgstapp-1.0.0.dylib",
                     "@rpath/libunknown.dylib", "/usr/lib/../../tmp/libunknown.dylib"):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "unresolved"):
                macos_linkage([name + " (compatibility version 1.0.0, current version 1.0.0)"], self.commands, self.runtime)

    def test_library_symlink_cannot_leave_the_declared_runtime(self):
        outside = Path(self.temp.name) / "outside.dylib"
        outside.write_bytes(b"Synthetic external library")
        self.library.unlink()
        self.library.symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "unresolved"):
            macos_linkage(self.linked, self.commands, self.runtime)


class DependencyNoticeFixtures(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="nmt-notice-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.stage = self.root / "stage"
        self.stage.mkdir()
        crate = self.root / "crate"
        crate.mkdir()
        (crate / "Cargo.toml").write_text("# Synthetic notice fixture\n")
        archive = self.root / "cargo/registry/cache/example/synthetic-1.0.0.crate"
        archive.parent.mkdir(parents=True)
        archive.write_bytes(b"Synthetic archive fixture; not Rust source")
        self.checksum = sha(archive)
        self.package = {"id": "synthetic", "name": "synthetic", "version": "1.0.0", "license": "fixture-only",
                        "source": "registry+https://example.invalid/index", "manifest_path": str(crate / "Cargo.toml")}
        (self.root / "Cargo.lock").write_text(
            '[[package]]\nname = "synthetic"\nversion = "1.0.0"\nsource = "registry+https://example.invalid/index"\n'
            f'checksum = "{self.checksum}"\n')
        self.notice = self.root / "tools/release/notices/synthetic-1.0.0/LICENSE"
        self.notice.parent.mkdir(parents=True)
        self.notice.write_bytes(b"Complete synthetic fixture notice\nNo real license grant.\n")
        self.manifest_path = self.notice.parent.parent / "manifest.json"
        self.entry = {"name": "synthetic", "version": "1.0.0", "license": "fixture-only", "crate_sha256": self.checksum,
                      "source_commit": "a" * 40, "files": [{"path": "synthetic-1.0.0/LICENSE", "sha256": sha(self.notice),
                                                            "source": "https://example.invalid/" + "a" * 40 + "/LICENSE"}]}
        self.save()

    def save(self):
        self.manifest_path.write_text(json.dumps({"schema": 1, "packages": [self.entry]}))

    def collect(self, features=""):
        with patch("build.run", side_effect=[json.dumps({"packages": [self.package]}), "synthetic v1.0.0|" + features]):
            dependency_notices(self.root, self.stage, "x86_64-unknown-linux-gnu", {"CARGO_HOME": str(self.root / "cargo")})
        return json.loads((self.stage / "dependencies.json").read_text())["packages"][0]

    def test_missing_packaged_notice_uses_exact_supplement_and_records_provenance(self):
        result = self.collect()
        self.assertEqual(result["notice_supplement"], self.entry)
        self.assertEqual((self.stage / result["notices"][0]).read_bytes(), self.notice.read_bytes())
        self.manifest_path.unlink()
        with self.assertRaisesRegex(ValueError, "Missing license text"):
            self.collect()

    def test_package_identity_license_and_file_hash_must_match(self):
        original = json.dumps(self.entry)
        for field, value in (("name", "another"), ("version", "2.0.0"), ("license", "another"), ("crate_sha256", "0" * 64)):
            with self.subTest(field=field):
                self.entry = json.loads(original)
                self.entry[field] = value
                self.save()
                with self.assertRaises(ValueError):
                    self.collect()
        self.entry = json.loads(original)
        self.save()
        self.notice.write_bytes(b"Changed notice")
        with self.assertRaisesRegex(ValueError, "has changed"):
            self.collect()

    def test_supplement_paths_are_confined_and_nonempty(self):
        original = json.dumps(self.entry)
        for name in ("../LICENSE", "/LICENSE", "another-1.0.0/LICENSE"):
            with self.subTest(path=name):
                self.entry["files"][0]["path"] = name
                self.save()
                with self.assertRaises(ValueError):
                    notice_supplement(self.root, self.package, self.checksum)
        self.entry = json.loads(original)
        self.save()
        self.notice.rename(self.root / "outside-LICENSE")
        self.notice.symlink_to(self.root / "outside-LICENSE")
        with self.assertRaisesRegex(ValueError, "Symlinks"):
            self.collect()
        self.entry["files"] = []
        self.save()
        with self.assertRaisesRegex(ValueError, "Empty"):
            self.collect()

    def test_packaged_notice_path_and_diagnostic_feature_exclusion_are_preserved(self):
        original = self.root / "crate/LICENSE"
        original.write_bytes(b"Original packaged fixture notice\n")
        self.manifest_path.write_text("invalid unused supplement manifest")
        result = self.collect()
        self.assertNotIn("notice_supplement", result)
        self.assertEqual((self.stage / result["notices"][0]).read_bytes(), original.read_bytes())
        with self.assertRaisesRegex(ValueError, "backend-test"):
            self.collect("backend-test")


class DeliveryFixtures(unittest.TestCase):
    def setUp(self):
        self.old_umask = os.umask(0o077)
        self.temp = tempfile.TemporaryDirectory(prefix="nmt-delivery-test-")
        self.root = Path(self.temp.name)
        self.stage = self.root / "input"
        self.spec = fixture(self.stage)

    def tearDown(self):
        self.temp.cleanup()
        os.umask(self.old_umask)

    def rejects(self, spec=None):
        if spec is not None:
            save(self.stage, spec)
        with self.assertRaises((ValueError, KeyError)):
            validate(self.stage, True)

    def test_fixture_cannot_enter_normal_package_cli(self):
        with self.assertRaises(ValueError):
            validate(self.stage)

    def test_tamper_and_extra_file_are_rejected(self):
        (self.stage / "assets/fonts/Manrope-Regular.ttf").write_bytes(b"changed")
        self.rejects()
        self.spec["files"]["assets/fonts/Manrope-Regular.ttf"] = sha(self.stage / "assets/fonts/Manrope-Regular.ttf")
        save(self.stage, self.spec)
        (self.stage / "secret.txt").write_text("synthetic forbidden payload")
        self.rejects()

    def test_traversal_and_symlink_are_rejected(self):
        self.spec["files"]["../outside"] = "0" * 64
        self.rejects(self.spec)
        del self.spec["files"]["../outside"]
        save(self.stage, self.spec)
        (self.stage / "bin/talk-desktop").unlink()
        (self.stage / "bin/talk-desktop").symlink_to("/bin/true")
        self.rejects()

    def test_missing_license_and_source_mismatch_are_rejected(self):
        name = REQUIRED_NOTICES[1]
        original = (self.stage / name).read_bytes()
        (self.stage / name).unlink()
        del self.spec["files"][name]
        self.rejects(self.spec)
        (self.stage / name).write_bytes(original)
        self.spec["files"][name] = sha(self.stage / name)
        self.spec["source"]["lock_sha256"] = "f" * 64
        save(self.stage, self.spec)
        with self.assertRaisesRegex(ValueError, "Lock/toolchain"):
            validate(self.stage, True)

    def test_diagnostic_feature_and_unsafe_version_are_rejected(self):
        self.spec["build"]["features"] = ["backend-test"]
        self.rejects(self.spec)
        self.spec["build"]["features"] = []
        self.spec["version"] = "0.0.0\nInjected: true"
        self.rejects(self.spec)

    def test_linux_elf_never_passes_as_mac_binary(self):
        self.spec["target"] = "aarch64-apple-darwin"
        deps = json.loads((self.stage / "dependencies.json").read_text())
        deps["target"] = self.spec["target"]
        (self.stage / "dependencies.json").write_text(json.dumps(deps))
        self.spec["files"]["dependencies.json"] = sha(self.stage / "dependencies.json")
        self.rejects(self.spec)

    def media_fixture(self):
        deps = json.loads((self.stage / "dependencies.json").read_text())
        for name, version in (("webrtc", "0.21.1"), ("rtc", "0.21.1"),
                              ("gstreamer", "0.25.4"), ("gstreamer-app", "0.25.2"),
                              ("gstreamer-video", "0.25.4")):
            deps["packages"].append({"name": name, "version": version, "license": "fixture-only",
                                     "source": "registry+https://example.invalid/index",
                                     "archive_sha256": "3" * 64, "features": [],
                                     "notices": [REQUIRED_NOTICES[0]]})
        self.spec["media"] = {"enabled": True, "engine": "webrtc", "version": "0.21.1",
                              "archive_sha256": "3" * 64, "codecs": "gstreamer"}
        self.save_dependencies(deps)
        return deps

    def save_dependencies(self, deps):
        (self.stage / "dependencies.json").write_text(json.dumps(deps))
        self.spec["files"]["dependencies.json"] = sha(self.stage / "dependencies.json")
        save(self.stage, self.spec)

    def test_enabled_media_matches_cargo_inventory_without_external_sdk(self):
        self.media_fixture()
        self.assertEqual(validate(self.stage, True)["media"], self.spec["media"])
        self.spec["media"]["archive_sha256"] = "4" * 64
        self.rejects(self.spec)
        self.spec["media"]["archive_sha256"] = "3" * 64
        self.spec["media"]["version"] = "0.21.0"
        self.rejects(self.spec)

    def test_media_requires_engine_codecs_and_dependency_notices(self):
        deps = self.media_fixture()
        for name in ("webrtc", "rtc", "gstreamer-app"):
            incomplete = {**deps, "packages": [p for p in deps["packages"] if p["name"] != name]}
            self.save_dependencies(incomplete)
            self.rejects()
        deps["packages"][1]["notices"] = []
        self.save_dependencies(deps)
        self.rejects()

    def test_obsolete_sdk_and_helper_payloads_are_rejected(self):
        deps = self.media_fixture()
        deps["packages"].append({"name": "libwebrtc", "version": "0.3.52",
                                 "license": "fixture-only", "notices": [REQUIRED_NOTICES[0]]})
        self.save_dependencies(deps)
        self.rejects(self.spec)
        (self.stage / "native-media.json").write_text('{"tag":"wrong"}')
        self.spec["files"]["native-media.json"] = sha(self.stage / "native-media.json")
        self.rejects(self.spec)

    def test_disabled_media_cannot_carry_helper(self):
        (self.stage / "bin/nmt-media-helper").write_bytes((self.stage / "bin/talk-desktop").read_bytes())
        self.spec["files"]["bin/nmt-media-helper"] = sha(self.stage / "bin/nmt-media-helper")
        self.rejects(self.spec)

    @unittest.skipUnless(shutil.which("dpkg-deb"), "Linux Debian assembler unavailable")
    def test_linux_archive_deb_roundtrip_and_repeatable_bytes(self):
        first, second = self.root / "first", self.root / "second"
        a = assemble(self.stage, first, True)
        b = assemble(self.stage, second, True)
        self.assertEqual(a, b)
        self.assertEqual({p.name: sha(p) for p in first.iterdir()}, {p.name: sha(p) for p in second.iterdir()})
        tar = next(first.glob("*.tar.gz"))
        with tarfile.open(tar) as archive:
            exe = next(m for m in archive.getmembers() if m.name.endswith("/bin/talk-desktop"))
            self.assertEqual(exe.mode, 0o755)
            self.assertEqual(hashlib.sha256(archive.extractfile(exe).read()).hexdigest(), self.spec["files"]["bin/talk-desktop"])
            self.assertFalse(any("helper" in m.name for m in archive.getmembers()))
            for name in REQUIRED_NOTICES:
                self.assertTrue(any(m.name.endswith("/" + name) for m in archive.getmembers()))
        import subprocess
        deb = next(first.glob("*.deb"))
        debian_version = subprocess.check_output(["dpkg-deb", "--field", str(deb), "Version"], text=True).strip()
        subprocess.run(["dpkg", "--compare-versions", debian_version, "lt", "0.0.0"], check=True)
        subprocess.run(["dpkg-deb", "--extract", str(deb), str(self.root / "installed")], check=True)
        self.assertEqual(sha(self.root / "installed/usr/lib/nextcloud-modern-talk/bin/talk-desktop"), self.spec["files"]["bin/talk-desktop"])
        self.assertEqual((self.root / "installed/usr/lib/nextcloud-modern-talk").stat().st_mode & 0o777, 0o755)
        if os.environ.get("NMT_DELIVERY_FIXTURE_OUTPUT"):
            shutil.copytree(first, Path(os.environ["NMT_DELIVERY_FIXTURE_OUTPUT"]) / "linux-fixture")

    def test_mac_bundle_metadata_modes_and_repeatable_zip(self):
        stage = self.root / "mac"
        spec = fixture(stage, "aarch64-apple-darwin")
        first, second = self.root / "mac-out", self.root / "mac-repeat"
        assemble(stage, first, True)
        assemble(stage, second, True)
        self.assertEqual({p.name: sha(p) for p in first.iterdir()}, {p.name: sha(p) for p in second.iterdir()})
        with zipfile.ZipFile(next(first.glob("*.zip"))) as archive:
            prefix = "NextCloud Modern Talk.app/Contents/"
            plist = plistlib.loads(archive.read(prefix + "Info.plist"))
            self.assertEqual(plist["CFBundleExecutable"], "talk-desktop")
            self.assertEqual(plist["LSMinimumSystemVersion"], "15.7")
            self.assertEqual((archive.getinfo(prefix + "MacOS/talk-desktop").external_attr >> 16) & 0o777, 0o755)
            self.assertFalse(any("Helpers/" in name for name in archive.namelist()))
        if os.environ.get("NMT_DELIVERY_FIXTURE_OUTPUT"):
            shutil.copytree(first, Path(os.environ["NMT_DELIVERY_FIXTURE_OUTPUT"]) / "mac-layout-fixture")



if __name__ == "__main__":
    unittest.main(verbosity=2)
