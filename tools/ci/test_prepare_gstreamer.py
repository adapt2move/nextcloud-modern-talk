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
                               side_effect=lambda *args, **kwargs: io.BytesIO(b"corrupt download")):
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                prepare_gstreamer.install(self.target)
        self.run.assert_not_called()
        self.assertFalse(self.env_file.exists())


class SdkRanges(unittest.TestCase):
    class Response(io.BytesIO):
        def __init__(self, data, status=200, headers=None):
            super().__init__(data)
            self.status = status
            self.headers = headers or {}

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.destination = self.root / "sdk.pkg"
        self.payload = bytes(range(256)) * (8 * 1024 * 1024 // 256) + b"sdk-tail"
        self.expected = hashlib.sha256(self.payload).hexdigest()
        self.seen = []
        self.corrupt = self.incomplete = self.wrong_range = False

    def response(self, request, timeout):
        self.assertEqual(timeout, 60)
        if request.get_method() == "HEAD":
            return self.Response(b"", headers={"Content-Length": str(len(self.payload)), "Accept-Ranges": "bytes"})
        header = request.get_header("Range")
        self.assertTrue(header.startswith("bytes="))
        start, end = map(int, header.removeprefix("bytes=").split("-"))
        self.seen.append((start, end))
        body = self.payload[start:end + 1]
        if self.corrupt and start == 0:
            body = b"!" + body[1:]
        if self.incomplete and start == 0:
            body = body[:-1]
        actual_range = f"bytes {start}-{end}/{len(self.payload)}"
        if self.wrong_range and start == 0:
            actual_range = f"bytes 1-{end}/{len(self.payload)}"
        return self.Response(body, status=206, headers={"Content-Range": actual_range})

    def download(self):
        with mock.patch.object(prepare_gstreamer.urllib.request, "urlopen", side_effect=self.response):
            prepare_gstreamer.download("https://sdk.example.test/sdk.pkg", self.destination, self.expected)

    def test_assemble_disjoint_ranges_and_verify_the_complete_archive(self):
        self.download()
        self.assertEqual(self.destination.read_bytes(), self.payload)
        self.assertEqual(len(self.seen), 4)
        ordered = sorted(self.seen)
        self.assertEqual(ordered[0][0], 0)
        self.assertEqual(ordered[-1][1], len(self.payload) - 1)
        self.assertTrue(all(left[1] + 1 == right[0] for left, right in zip(ordered, ordered[1:])))
        self.assertEqual(list(self.root.iterdir()), [self.destination])

    def test_corruption_incomplete_and_wrong_ranges_leave_no_accepted_archive(self):
        for kind, message in [("corrupt", "checksum mismatch"), ("incomplete", "incomplete byte range"),
                              ("wrong_range", "unexpected byte range")]:
            with self.subTest(kind=kind):
                setattr(self, kind, True)
                with self.assertRaisesRegex(ValueError, message):
                    self.download()
                setattr(self, kind, False)
                self.assertEqual(list(self.root.iterdir()), [])

    def test_missing_range_support_uses_one_verified_stream(self):
        def response(request, timeout):
            if not isinstance(request, str):
                self.assertEqual(request.get_method(), "HEAD")
                return self.Response(b"", headers={"Content-Length": str(len(self.payload))})
            return self.Response(self.payload)
        with mock.patch.object(prepare_gstreamer.urllib.request, "urlopen", side_effect=response) as opened:
            prepare_gstreamer.download("https://sdk.example.test/sdk.pkg", self.destination, self.expected)
        self.assertEqual(opened.call_count, 2)
        self.assertEqual(self.destination.read_bytes(), self.payload)

    def test_oversized_metadata_stops_before_fetching_a_body(self):
        with mock.patch.object(prepare_gstreamer.urllib.request, "urlopen",
                               return_value=self.Response(b"", headers={"Content-Length": str(1024 ** 3 + 1), "Accept-Ranges": "bytes"})) as opened:
            with self.assertRaisesRegex(ValueError, "SDK archive too large"):
                prepare_gstreamer.download("https://sdk.example.test/sdk.pkg", self.destination, self.expected)
        self.assertEqual(opened.call_count, 1)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_expired_transfer_removes_parts_and_never_creates_an_archive(self):
        now = [0]
        def response(request, timeout):
            if request.get_method() != "HEAD":
                now[0] = 1201
            return self.response(request, timeout)
        with mock.patch.object(prepare_gstreamer.time, "monotonic", side_effect=lambda: now[0]), \
             mock.patch.object(prepare_gstreamer.urllib.request, "urlopen", side_effect=response):
            with self.assertRaisesRegex(ValueError, "time limit"):
                prepare_gstreamer.download("https://sdk.example.test/sdk.pkg", self.destination, self.expected)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_failed_download_cannot_remove_an_existing_destination(self):
        self.destination.write_bytes(b"existing download")
        with self.assertRaises(FileExistsError):
            self.download()
        self.assertEqual(self.destination.read_bytes(), b"existing download")

    def test_rate_limit_is_propagated_without_starting_body_requests(self):
        error = prepare_gstreamer.urllib.error.HTTPError("https://sdk.example.test/sdk.pkg", 429, "Busy", {}, None)
        with mock.patch.object(prepare_gstreamer.urllib.request, "urlopen", side_effect=error) as opened:
            with self.assertRaises(prepare_gstreamer.urllib.error.HTTPError):
                prepare_gstreamer.download("https://sdk.example.test/sdk.pkg", self.destination, self.expected)
        self.assertEqual(opened.call_count, 1)
        self.assertEqual(list(self.root.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
