"""Room settings must use the participant whose credential was supplied."""
import base64
import io
import json
import unittest
from unittest.mock import patch

from ice_settings import fetch_settings


class SignalingSettingsTests(unittest.TestCase):
    def test_direct_room_settings_use_the_admitted_caller(self):
        for user in ("lab_outsider", "lab_alice"):
            with self.subTest(user=user):
                response = io.BytesIO(json.dumps({"ocs": {
                    "meta": {"status": "ok"}, "data": {"signalingMode": "external"},
                }}).encode())
                with patch("urllib.request.build_opener") as factory:
                    factory.return_value.open.return_value = response
                    result = fetch_settings("https://127.0.0.1:8785", "fixture-only", "room", user)
                    request = factory.return_value.open.call_args.args[0]
                self.assertEqual(result, {"signalingMode": "external"})
                self.assertEqual(request.get_header("Authorization"), "Basic " +
                    base64.b64encode((user + ":fixture-only").encode()).decode())
                self.assertEqual(request.full_url,
                    "https://127.0.0.1:8785/ocs/v2.php/apps/spreed/api/v3/signaling/settings?token=room")

    def test_existing_ice_probe_defaults_to_alice(self):
        response = io.BytesIO(b'{"ocs":{"meta":{"status":"ok"},"data":{}}}')
        with patch("urllib.request.build_opener") as factory:
            factory.return_value.open.return_value = response
            fetch_settings("https://127.0.0.1:8785", "fixture-only", "room")
            request = factory.return_value.open.call_args.args[0]
        self.assertEqual(request.get_header("Authorization"), "Basic " +
            base64.b64encode(b"lab_alice:fixture-only").decode())


if __name__ == "__main__":
    unittest.main()
