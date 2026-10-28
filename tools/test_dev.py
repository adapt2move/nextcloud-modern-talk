"""Signing orchestration tests; no user keychain or trust settings are touched."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import dev

FINGERPRINT = "A" * 40


@unittest.skipUnless(hasattr(os, "getuid"), "Unix private-file permission checks")
class DevelopmentSigning(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def configured(self):
        path = self.root / "signing.json"
        path.write_text(json.dumps({"schema": 1, "identifier": dev.IDENTIFIER,
                                    "certificate_sha1": FINGERPRINT}))
        path.chmod(0o600)
        return path

    def test_restarts_and_rebuilds_reuse_the_configured_certificate_and_identifier(self):
        self.configured()
        with patch.object(dev, "run", return_value=FINGERPRINT) as run:
            for binary in [self.root / "first build", self.root / "rebuilt app"]:
                dev.sign(binary, self.root)
            signing = [c.args[0] for c in run.call_args_list if c.args[0][0] == "codesign" and "--sign" in c.args[0]]
            self.assertEqual(len(signing), 2)
            for command in signing:
                self.assertEqual(command[command.index("--sign") + 1], FINGERPRINT)
                self.assertEqual(command[command.index("--identifier") + 1], dev.IDENTIFIER)

    def test_an_unavailable_configured_certificate_never_creates_a_replacement(self):
        self.configured()
        with patch.object(dev.platform, "system", return_value="Darwin"), patch.object(dev, "run", return_value="0 valid identities found") as run:
            with self.assertRaisesRegex(ValueError, "unavailable"):
                dev.setup_signing(self.root)
        self.assertEqual(run.call_count, 1)

    def test_setup_keeps_the_key_private_and_limits_trust_and_key_access(self):
        calls, staged = [], []
        trusted = False

        def execute(command):
            nonlocal trusted
            command = list(map(str, command))
            calls.append(command)
            if command[:2] == ["security", "find-identity"]:
                return f'1) {FINGERPRINT} "{dev.CERTIFICATE}"' if trusted else ""
            if command[:2] == ["security", "default-keychain"]:
                return json.dumps(str(self.root / "login.keychain-db"))
            if command[:2] == ["openssl", "req"]:
                key = Path(command[command.index("-keyout") + 1])
                key.write_text("synthetic private key")
                self.assertEqual(key.stat().st_mode & 0o777, 0o600)
                self.assertEqual(key.parent.stat().st_mode & 0o777, 0o700)
                staged.append(key)
            if command[:2] == ["openssl", "x509"]:
                return "sha1 Fingerprint=" + ":".join(["AA"] * 20)
            if command[:2] == ["security", "add-trusted-cert"]:
                trusted = True
            return ""

        with patch.object(dev.platform, "system", return_value="Darwin"), patch.object(dev, "run", side_effect=execute):
            self.assertEqual(dev.setup_signing(self.root), FINGERPRINT)
        imported = next(c for c in calls if c[:2] == ["security", "import"])
        self.assertIn("-x", imported)
        self.assertEqual(imported[-2:], ["-T", "/usr/bin/codesign"])
        self.assertNotIn("-A", imported)
        self.assertNotIn("-P", imported)
        trust = next(c for c in calls if c[:2] == ["security", "add-trusted-cert"])
        self.assertEqual(trust[trust.index("-p") + 1], "codeSign")
        self.assertNotIn("-d", trust)
        self.assertTrue(staged)
        self.assertTrue(all(not key.exists() for key in staged))
        self.assertNotIn("private key", (self.root / "signing.json").read_text())

    def test_failed_setup_removes_staging_and_writes_no_success_configuration(self):
        def execute(command):
            if list(command)[:2] == ["security", "default-keychain"]:
                return json.dumps(str(self.root / "login.keychain-db"))
            if list(command)[:2] == ["openssl", "x509"]:
                return "sha1 Fingerprint=" + FINGERPRINT
            if list(command)[:2] == ["security", "import"]:
                raise ValueError("import denied")
            return ""

        with patch.object(dev.platform, "system", return_value="Darwin"), patch.object(dev, "run", side_effect=execute):
            with self.assertRaisesRegex(ValueError, "denied"):
                dev.setup_signing(self.root)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_non_macos_keeps_native_cargo_run_and_app_arguments(self):
        with patch.object(dev.platform, "system", return_value="Linux"), patch.object(dev.cargo, "main", return_value=0) as cargo:
            self.assertEqual(dev.main(["--ui-scenario", "chat"]), 0)
            cargo.assert_called_once_with(["run", "--locked", "-p", "talk-desktop", "--", "--ui-scenario", "chat"])

    def test_macos_builds_debug_then_signs_and_passes_literal_arguments_to_exec(self):
        literal = "a 'quote' with $(literal)"
        with patch.object(dev.platform, "system", return_value="Darwin"), \
             patch.object(dev, "private_root", return_value=self.root), \
             patch.object(dev, "identity", return_value=FINGERPRINT), \
             patch.object(dev.subprocess, "run") as build, \
             patch.object(dev, "sign") as sign, \
             patch.object(dev.os, "chdir"), \
             patch.object(dev.os, "execve") as execute, \
             patch.object(dev.cargo, "environment", return_value={"test": "environment"}):
            dev.main(["--ui-scenario", literal])
        command = build.call_args.args[0]
        self.assertEqual(command[2:], ["build", "--locked", "-p", "talk-desktop"])
        binary = dev.ROOT / "target/debug/talk-desktop"
        sign.assert_called_once_with(binary)
        execute.assert_called_once_with(binary, [str(binary), "--ui-scenario", literal], {"test": "environment"})


if __name__ == "__main__":
    unittest.main()
