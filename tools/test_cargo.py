#!/usr/bin/env python3
"""Environment failures, flag precedence and literal Task argument forwarding."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("project_cargo", ROOT / "tools/cargo.py")
cargo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo)


class EnvironmentTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.sdk = self.root / "SDK with spaces"
        (self.sdk / "lib/pkgconfig").mkdir(parents=True)
        (self.sdk / "lib/pkgconfig/gstreamer-1.0.pc").touch()
        self.env = {"PATH": "synthetic", "TMPDIR": str(self.root), "PKG_CONFIG_PATH": "stale-sdk"}

    def which(self, name, **_):
        return "/synthetic/" + name if name in {"pkg-config", "brew"} else None

    def query(self, argv, env):
        if "--prefix" in argv:
            return str(self.sdk)
        if "--atleast-version=1.24" in argv:
            return ""
        if str(self.sdk / "lib/pkgconfig") in env.get("PKG_CONFIG_PATH", ""):
            return str(self.sdk / "lib")
        return None

    def environment(self, source=None, query=None):
        with patch.object(cargo.shutil, "which", side_effect=self.which), \
             patch.object(cargo, "query", side_effect=query or self.query), \
             patch.object(cargo, "FRAMEWORK", self.root / "missing-framework"):
            return cargo.environment(["build"], self.env if source is None else source, "Darwin")

    def test_non_macos_and_non_native_commands_do_not_probe_or_modify(self):
        with patch.object(cargo, "macos_gstreamer", side_effect=AssertionError("unexpected SDK probe")):
            for system in ("Linux", "Windows"):
                self.assertEqual(cargo.environment(["build"], self.env, system), self.env)
            self.assertEqual(cargo.environment(["metadata"], self.env, "Darwin"), self.env)

    def test_stale_sdk_path_falls_back_and_preserves_flags_with_spaces(self):
        env = self.environment(dict(self.env, RUSTFLAGS="-D warnings"))
        self.assertEqual(env["PKG_CONFIG_PATH"], str(self.sdk / "lib/pkgconfig") + os.pathsep + "stale-sdk")
        expected = ["-D", "warnings", "-C", "link-arg=-Wl,-rpath," + str(self.sdk / "lib")]
        self.assertEqual(env["CARGO_ENCODED_RUSTFLAGS"].split("\x1f"), expected)
        self.assertEqual(self.environment(env)["CARGO_ENCODED_RUSTFLAGS"], env["CARGO_ENCODED_RUSTFLAGS"])
        self.assertNotIn("CARGO_ENCODED_RUSTFLAGS", self.env)

    def test_encoded_flags_take_precedence_and_temporary_alias_is_canonical(self):
        alias = self.root / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        env = self.environment(dict(self.env, TMPDIR=str(alias), RUSTFLAGS="ignored", CARGO_ENCODED_RUSTFLAGS="-D\x1fwarnings"))
        self.assertNotIn("ignored", env["CARGO_ENCODED_RUSTFLAGS"])
        self.assertEqual(env["TMPDIR"], str(self.root))

    def test_explicit_sdk_is_not_replaced_by_homebrew(self):
        explicit = self.root / "explicit/lib"
        explicit.mkdir(parents=True)
        def selected(argv, env):
            if "--variable=libdir" in argv:
                return str(explicit)
            return self.query(argv, env)
        env = self.environment(query=selected)
        self.assertEqual(env["PKG_CONFIG_PATH"], "stale-sdk")
        self.assertIn(str(explicit), env["CARGO_ENCODED_RUSTFLAGS"])

    def test_missing_or_old_sdk_reports_failure_instead_of_running_cargo(self):
        with self.assertRaisesRegex(ValueError, "development files were not found"):
            self.environment(query=lambda argv, env: None)
        def old(argv, env):
            if "--atleast-version=1.24" in argv:
                return None
            return self.query(argv, env)
        with self.assertRaisesRegex(ValueError, "older than 1.24"):
            self.environment(query=old)
        with patch.object(cargo.shutil, "which", return_value=None):
            with self.assertRaisesRegex(ValueError, "pkg-config is missing"):
                cargo.environment(["build"], self.env, "Darwin")


@unittest.skipIf(os.name == "nt", "Fake executable fixtures use POSIX shebangs; Windows remains compile-only")
class CommandTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / "lib").mkdir()
        self.record = self.root / "arguments.json"
        for name in ("python3", "python"):
            (self.root / name).symlink_to(sys.executable)
        fake = self.root / "cargo"
        fake.write_text("#!" + sys.executable + "\nimport os,sys,json\nfrom pathlib import Path\n"
                        "Path(os.environ['CARGO_TEST_RECORD']).write_text(json.dumps(sys.argv[1:]))\n"
                        "sys.exit(int(os.environ.get('CARGO_TEST_EXIT','0')))\n")
        fake.chmod(0o700)
        pkg = self.root / "pkg-config"
        pkg.write_text("#!" + sys.executable + "\nimport sys\n"
                       "print(" + repr(str(self.root / "lib")) + ") if '--variable=libdir' in sys.argv else None\n")
        pkg.chmod(0o700)
        self.env = dict(os.environ, PATH=str(self.root), TMPDIR=str(self.root), CARGO_TEST_RECORD=str(self.record))

    def test_wrapper_preserves_arguments_and_exit_status(self):
        arguments = ["build", "--locked", "--target-dir", "space ' quoted $(literal)"]
        result = subprocess.run([sys.executable, str(ROOT / "tools/cargo.py"), *arguments],
                                env=dict(self.env, CARGO_TEST_EXIT="17"), capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(json.loads(self.record.read_text()), arguments)

    @unittest.skipUnless(shutil.which("task"), "Go Task is optional in CI; the Cargo starter has no Task dependency")
    def test_task_dev_and_build_preserve_profiles_and_literal_arguments(self):
        task = shutil.which("task")
        sentinel = self.root / "must-not-be-created"
        literal = "a 'quote' with $(touch " + str(sentinel) + ")"
        # Task dispatch is tested without accessing the operator's macOS signing
        # keychain. test_dev covers the real development runner separately.
        for name in ("python3", "python"):
            executable = self.root / name
            executable.unlink()  # Remove the fixture symlink, never its target.
            executable.write_text("#!" + sys.executable + "\nimport os,sys,json\nfrom pathlib import Path\n"
                "if sys.argv[1] == 'tools/dev.py':\n"
                " Path(os.environ['CARGO_TEST_RECORD']).write_text(json.dumps(sys.argv[1:]))\n"
                "else:\n os.execv(" + repr(sys.executable) + ", [" + repr(sys.executable) + ", *sys.argv[1:]])\n")
            executable.chmod(0o700)
        for name, expected in (
            ("dev", ["tools/dev.py", "--ui-scenario", literal]),
            ("build", ["build", "--locked", "--release", "--no-default-features", "-p", "talk-desktop", "--target-dir", literal]),
        ):
            flag = "--ui-scenario" if name == "dev" else "--target-dir"
            result = subprocess.run([task, name, "--", flag, literal], cwd=ROOT / "docs", env=self.env, capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(self.record.read_text()), expected)
            self.assertFalse(sentinel.exists())


if __name__ == "__main__":
    unittest.main()
