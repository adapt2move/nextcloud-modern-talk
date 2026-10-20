#!/usr/bin/env python3
"""Regression checks for private native evidence output ownership; no service run."""
from pathlib import Path
import os
import subprocess
import sys
import tempfile
import unittest
from native_lock_race import validate_outputs


class NativeEvidencePaths(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="nmt-native-evidence-paths-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.binary = self.root / "test-binary"
        self.manifest = self.root / "source.json"
        self.binary.write_bytes(b"not executed")
        self.manifest.write_text("{}")
        self.report = self.root / "report.json"
        self.raw = self.root / "rust.txt"

    def test_distinct_paths_do_not_open_outputs(self):
        validate_outputs(self.binary, self.report, self.manifest, self.raw)
        self.assertFalse(self.report.exists())
        self.assertFalse(self.raw.exists())

    def test_report_raw_or_input_collision_rejected(self):
        for raw in [self.report, self.binary, self.manifest]:
            with self.subTest(raw=raw.name), self.assertRaises(ValueError):
                validate_outputs(self.binary, self.report, self.manifest, raw)

    def test_existing_symlink_hardlink_and_raw_outputs_preserved(self):
        for kind in ["existing", "symlink", "hardlink"]:
            raw = self.root / kind
            if kind == "symlink":
                raw.symlink_to(self.binary)
            elif kind == "hardlink":
                os.link(self.binary, raw)
            else:
                raw.write_bytes(b"retained evidence")
            before = raw.read_bytes()
            with self.subTest(kind=kind), self.assertRaises((ValueError, FileExistsError)):
                validate_outputs(self.binary, self.report, self.manifest, raw)
            self.assertEqual(raw.read_bytes(), before)

    def test_parent_stdout_collision_rejected_before_service_or_rust(self):
        controller = Path(__file__).with_name("native_lock_race.py")
        with self.raw.open("wb") as output:
            result = subprocess.run([sys.executable, str(controller), "--binary", str(self.binary),
                                     "--source-manifest", str(self.manifest), "--report", str(self.report),
                                     "--raw-log", str(self.raw)], stdout=output, stderr=subprocess.PIPE)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"native evidence aliases controller output", result.stderr)
        self.assertFalse(self.report.exists())
        self.assertEqual(self.raw.read_bytes(), b"")


if __name__ == "__main__":
    unittest.main()
