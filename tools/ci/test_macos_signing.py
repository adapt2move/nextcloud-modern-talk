#!/usr/bin/env python3
"""Exercise the disposable signing-keychain boundary with synthetic inputs only."""
import base64
import os
from pathlib import Path
import tempfile
import unittest
import subprocess
import importlib.util
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

    def test_internal_import_does_not_trust_a_root_or_accept_a_different_certificate(self):
        for fingerprint in ("A" * 40, "B" * 40):
            with tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                env = {"RUNNER_TEMP": temp, "GITHUB_ENV": str(root / "env"),
                       "MACOS_CERTIFICATE_P12_BASE64": base64.b64encode(b"synthetic-p12").decode(),
                       "MACOS_CERTIFICATE_PASSWORD": "synthetic-password", "MACOS_SIGNING_MODE": "internal",
                       "MACOS_SIGNING_CERTIFICATE": "A" * 40}
                commands = []
                def run(command):
                    commands.append(command)
                    return ('1) ' + fingerprint + ' "Synthetic Alpha" (CSSMERR_TP_NOT_TRUSTED)\n'
                            if command[1] == "find-identity" else "")
                with patch.dict(os.environ, env, clear=True), patch.object(macos_signing, "run", side_effect=run), patch.object(macos_signing, "compatible_identity", side_effect=lambda path, _: path):
                    if fingerprint == "A" * 40:
                        macos_signing.prepare()
                        self.assertIn("NMT_MACOS_SIGNING_MODE=internal", (root / "env").read_text())
                        macos_signing.cleanup(next(p for p in root.iterdir() if p.is_dir()))
                    else:
                        with self.assertRaisesRegex(ValueError, "matching persistent"):
                            macos_signing.prepare()
                        self.assertFalse(list(root.glob("nmt-signing-*")))
                self.assertFalse(any("add-trusted-cert" in c or "default-keychain" in c or "list-keychains" in c for c in commands))

    def test_internal_conversion_preserves_the_actual_certificate_and_private_key(self):
        spec = importlib.util.spec_from_file_location("internal_identity", Path(__file__).parents[1] / "release/internal_identity.py")
        identity = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(identity)
        with tempfile.TemporaryDirectory() as temp:
            root, fingerprint = identity.prepare(Path(temp) / "identity")
            original = (root / "identity.p12").read_bytes()
            password = (root / "password").read_text()
            converted = macos_signing.compatible_identity(root / "identity.p12", password)
            self.assertEqual((root / "identity.p12").read_bytes(), original)
            self.assertEqual(converted.stat().st_mode & 0o777, 0o600)
            environment = dict(os.environ, NMT_P12_TEST_PASSWORD=password)
            def extract(path, option):
                return subprocess.run(["openssl", "pkcs12", "-in", str(path), "-passin", "env:NMT_P12_TEST_PASSWORD", option, "-nodes"], capture_output=True, env=environment, check=True, timeout=30).stdout
            public = extract(converted, "-nokeys")
            digest = identity.run(["openssl", "x509", "-noout", "-fingerprint", "-sha1"], data=public).decode()
            self.assertEqual(digest.split("=", 1)[-1].replace(":", "").strip().upper(), fingerprint)
            # Compare the actual private key's public part without exposing it.
            before = identity.run(["openssl", "pkey", "-pubout", "-outform", "DER"], data=extract(root / "identity.p12", "-nocerts"))
            after = identity.run(["openssl", "pkey", "-pubout", "-outform", "DER"], data=extract(converted, "-nocerts"))
            self.assertEqual(before, after)
            info = subprocess.run(["openssl", "pkcs12", "-in", str(converted), "-passin", "env:NMT_P12_TEST_PASSWORD", "-info", "-noout"], capture_output=True, env=environment, check=True, timeout=30)
            self.assertIn(b"MAC: sha1", info.stderr)
            self.assertIn(b"Iteration 100000", info.stderr)
            self.assertFalse(any(b"PRIVATE KEY" in path.read_bytes() for path in root.iterdir()),
                             "a decrypted private key was written to disk")

    def test_provider_failure_identifies_the_operation_without_arguments_or_stderr(self):
        result = subprocess.CompletedProcess([], 1, "", "synthetic-password and private provider metadata")
        with patch.object(macos_signing.subprocess, "run", return_value=result):
            with self.assertRaises(ValueError) as caught:
                macos_signing.run(["security", "import", "/private/input", "-P", "synthetic-password"])
        self.assertIn("import, exit 1", str(caught.exception))
        self.assertNotIn("synthetic-password", str(caught.exception))
        self.assertNotIn("/private/input", str(caught.exception))
        self.assertNotIn("metadata", str(caught.exception))

    def test_failed_internal_conversion_removes_the_keychain_without_import_or_success(self):
        with tempfile.TemporaryDirectory() as temp:
            env = {"RUNNER_TEMP": temp, "GITHUB_ENV": str(Path(temp) / "env"),
                   "MACOS_CERTIFICATE_P12_BASE64": base64.b64encode(b"synthetic-p12").decode(),
                   "MACOS_CERTIFICATE_PASSWORD": "synthetic-password", "MACOS_SIGNING_MODE": "internal",
                   "MACOS_SIGNING_CERTIFICATE": "A" * 40}
            commands = []
            def run(command):
                commands.append(command)
                # Simulate the disposable keychain so cleanup exercises deletion.
                if command[1] == "create-keychain":
                    Path(command[-1]).touch()
                return ""
            with patch.dict(os.environ, env, clear=True), patch.object(macos_signing, "run", side_effect=run), patch.object(macos_signing, "compatible_identity", side_effect=ValueError("conversion failed")):
                with self.assertRaisesRegex(ValueError, "conversion failed"):
                    macos_signing.prepare()
            self.assertTrue(any(c[1] == "delete-keychain" for c in commands))
            self.assertFalse(any(c[1] == "import" for c in commands))
            self.assertFalse(list(Path(temp).iterdir()))

    def test_cleanup_rejects_an_unowned_directory(self):
        with tempfile.TemporaryDirectory() as temp, patch.dict(os.environ, {"RUNNER_TEMP": temp}):
            with self.assertRaises(ValueError):
                macos_signing.cleanup(Path(temp))


if __name__ == "__main__":
    unittest.main()
