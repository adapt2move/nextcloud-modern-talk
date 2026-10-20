#!/usr/bin/env python3
"""Synthetic MSI layout checks; these do not execute Windows Installer."""
import hashlib
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import prepare_gstreamer


class WindowsSdkInstall(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="gstreamer-fixture-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.install_dir = self.root / "gstreamer"
        self.sdk = self.install_dir / "1.0" / "msvc_x86_64"
        self.path_file = self.root / "github-path"
        self.env_file = self.root / "github-env"
        self.write_pkg_config = True
        self.exit_codes = iter((0, 3010))
        self.target = "x86_64-pc-windows-msvc"
        payload = b"synthetic MSI download"
        digest = hashlib.sha256(payload).hexdigest()
        url, packages = prepare_gstreamer.PACKAGES[self.target]
        patches = [
            mock.patch.dict(os.environ, RUNNER_TEMP=str(self.root),
                            GITHUB_PATH=str(self.path_file), GITHUB_ENV=str(self.env_file)),
            mock.patch.object(prepare_gstreamer.platform, "system", return_value="Windows"),
            mock.patch.dict(prepare_gstreamer.PACKAGES,
                            {self.target: (url, [(name, digest) for name, _ in packages])}),
            mock.patch.object(prepare_gstreamer.urllib.request, "urlopen",
                              side_effect=lambda *args, **kwargs: io.BytesIO(payload)),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)
        installer = mock.patch.object(prepare_gstreamer.subprocess, "run", side_effect=self.install_msi)
        self.run = installer.start()
        self.addCleanup(installer.stop)

    def install_msi(self, command, timeout):
        self.assertEqual(timeout, 900)
        self.assertIn("ADDLOCAL=ALL", command)
        install_dir = Path(next(arg.split("=", 1)[1] for arg in command if arg.startswith("INSTALLDIR=")))
        self.assertEqual(install_dir, self.install_dir)
        if "/L*v" in command:
            Path(command[command.index("/L*v") + 1]).write_text("synthetic installer log\n")
        code = next(self.exit_codes)
        if code in (0, 3010) and self.write_pkg_config and "-devel-" in command[2]:
            # Released Cerbero 1.26.11 MSI layout: INSTALLDIR/Version/Architecture.
            pc = install_dir / "1.0/msvc_x86_64/lib/pkgconfig/gstreamer-1.0.pc"
            pc.parent.mkdir(parents=True)
            pc.write_text("Version: 1.26.11\n")
        return subprocess.CompletedProcess(command, code)

    def test_released_layout_exports_sdk_prefix_and_retains_logs(self):
        prepare_gstreamer.install(self.target)
        self.assertEqual(self.run.call_count, 2)
        self.assertEqual(self.path_file.read_text(), str(self.sdk / "bin") + "\n")
        self.assertEqual(self.env_file.read_text(),
                         f"PKG_CONFIG_PATH={self.sdk / 'lib/pkgconfig'}\nGSTREAMER_ROOT_X86_64={self.sdk}\n")
        self.assertEqual(len(list((self.root / "gstreamer-installer-logs").glob("*.log"))), 2)

    def test_missing_nested_pc_rejects_flat_prefix_and_exports_nothing(self):
        self.write_pkg_config = False
        flat_pc = self.install_dir / "lib/pkgconfig/gstreamer-1.0.pc"
        flat_pc.parent.mkdir(parents=True)
        flat_pc.write_text("unrelated SDK\n")
        with self.assertRaises(ValueError) as failure:
            prepare_gstreamer.install(self.target)
        self.assertIn(str(self.sdk / "lib/pkgconfig/gstreamer-1.0.pc"), str(failure.exception))
        self.assertIn(str(self.root / "gstreamer-installer-logs"), str(failure.exception))
        self.assertFalse(self.path_file.exists())
        self.assertFalse(self.env_file.exists())

    def test_failed_installer_reports_package_exit_code_and_retained_log(self):
        self.exit_codes = iter((1603,))
        with self.assertRaisesRegex(RuntimeError, r"gstreamer-1.0-msvc-x86_64-1.26.11.msi exited 1603; MSI log:"):
            prepare_gstreamer.install(self.target)
        self.assertEqual(self.run.call_count, 1)
        self.assertEqual(len(list((self.root / "gstreamer-installer-logs").glob("*.log"))), 1)
        self.assertFalse(self.env_file.exists())

    def test_bad_checksum_never_runs_installer(self):
        with mock.patch.object(prepare_gstreamer.urllib.request, "urlopen",
                               return_value=io.BytesIO(b"corrupt download")):
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                prepare_gstreamer.install(self.target)
        self.run.assert_not_called()
        self.assertFalse(self.env_file.exists())


if __name__ == "__main__":
    unittest.main()
