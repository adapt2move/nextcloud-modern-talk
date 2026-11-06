#!/usr/bin/env python3
"""Real ephemeral key generation; persistent identity and no-overwrite boundaries."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import internal_identity as identity


class IdentityTests(unittest.TestCase):
    def test_backup_reuse_preserves_the_actual_certificate_and_encrypted_key(self):
        with tempfile.TemporaryDirectory() as temp:
            root, fingerprint = identity.prepare(Path(temp) / "signing")
            before = {p.name: p.read_bytes() for p in root.iterdir()}
            self.assertEqual(identity.prepare(root), (root, fingerprint))
            self.assertEqual({p.name: p.read_bytes() for p in root.iterdir()}, before)
            self.assertEqual(set(before), {"identity.json", "identity.p12", "certificate.pem", "password"})
            self.assertEqual(root.stat().st_mode & 0o777, 0o700)
            self.assertTrue(all(p.stat().st_mode & 0o777 == 0o600 for p in root.iterdir()))
            self.assertNotIn(b"PRIVATE KEY", before["identity.p12"])
            self.assertNotIn(before["password"], before["identity.json"])

    def test_incomplete_or_accessible_backups_are_not_silently_replaced(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "signing"
            root.mkdir(mode=0o700)
            (root / "partial").write_text("synthetic")
            with self.assertRaisesRegex(ValueError, "incomplete"):
                identity.prepare(root)
            root.chmod(0o755)
            with self.assertRaisesRegex(ValueError, "owner-private"):
                identity.prepare(root)

    def test_existing_repository_signer_is_never_overwritten(self):
        def run(command, **_):
            return json.dumps([{"name": "MACOS_CERTIFICATE_PASSWORD"}]).encode() if command[1] == "secret" else b"[]"
        with patch.object(identity, "run", side_effect=run) as calls:
            with self.assertRaisesRegex(ValueError, "refusing to rotate"):
                identity.install(Path("/unused"), "A" * 40, "synthetic/repository")
        self.assertTrue(all("set" not in call.args[0] for call in calls.call_args_list))


if __name__ == "__main__":
    unittest.main()
