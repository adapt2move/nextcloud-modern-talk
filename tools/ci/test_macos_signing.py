#!/usr/bin/env python3
"""Exercise the disposable signing-keychain boundary with synthetic inputs only."""
import base64
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import macos_signing


class SigningTests(unittest.TestCase):
    def test_missing_or_malformed_credentials_do_not_start_a_provider(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(macos_signing, "run") as run:
            with self.assertRaisesRegex(ValueError, "Publishing needs"):
                macos_signing.prepare()
            run.assert_not_called()
        for cert in ["invalid base64!", "A" * 350_001, ""]:
            with self.assertRaises(ValueError):
                macos_signing.credentials({"MACOS_CERTIFICATE_P12_BASE64": cert,
                    "MACOS_CERTIFICATE_PASSWORD": "synthetic", "MACOS_SIGNING_TEAM": "ABCDEFGHIJ"})

    def test_import_never_persists_the_certificate_or_password_in_workflow_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            env = {"RUNNER_TEMP": temp, "GITHUB_ENV": str(root / "env"),
                   "MACOS_CERTIFICATE_P12_BASE64": base64.b64encode(b"synthetic-p12").decode(),
                   "MACOS_CERTIFICATE_PASSWORD": "synthetic-password", "MACOS_SIGNING_TEAM": "ABCDEFGHIJ"}
            commands = []
            def run(command):
                commands.append(command)
                if command[1] == "find-identity":
                    return '1) ' + 'A' * 40 + ' "Developer ID Application: Synthetic (ABCDEFGHIJ)"\n'
                return ""
            with patch.dict(os.environ, env, clear=True), patch.object(macos_signing, "run", side_effect=run):
                macos_signing.prepare()
                output = (root / "env").read_text()
                self.assertIn("NMT_MACOS_REQUIRE_SIGNED=1", output)
                self.assertNotIn("synthetic-password", output)
                self.assertNotIn(env["MACOS_CERTIFICATE_P12_BASE64"], output)
                self.assertFalse(list(root.rglob("*.p12")))
                self.assertFalse(any("list-keychains" in c or "default-keychain" in c for c in commands))
                directory = next(p for p in root.iterdir() if p.is_dir())
                self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
                macos_signing.cleanup(directory)
                self.assertFalse(directory.exists())

    def test_cleanup_rejects_an_unowned_directory(self):
        with tempfile.TemporaryDirectory() as temp, patch.dict(os.environ, {"RUNNER_TEMP": temp}):
            with self.assertRaises(ValueError):
                macos_signing.cleanup(Path(temp))


if __name__ == "__main__":
    unittest.main()
