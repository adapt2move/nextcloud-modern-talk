"""Negative controls for the macOS device-call receiver verdict."""
import asyncio
import json
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import unittest

from macos_device_call import Evidence, Lab, ProbeError, Subscription, run_cases


class ReceiverTests(unittest.TestCase):
    def test_retry_needs_new_packets_and_the_test_tone_after_quiet(self):
        baseline = Evidence("mic-stalled", True)
        tone = [{"id": 1, "tone": True, "decoder": True, "packets": 200}]
        self.assertFalse(baseline.sample(1, [], False, 0))
        self.assertFalse(baseline.sample(1, tone, False, 2))
        self.assertFalse(baseline.sample(1, [], False, 3))
        self.assertTrue(baseline.sample(1, [], False, 5))

        retry = Evidence("mic-retried", True)
        self.assertFalse(retry.sample(3, tone, False, 5))
        self.assertFalse(retry.sample(3, tone, False, 7))
        tone[0]["packets"] += 10
        self.assertFalse(retry.sample(1, tone, False, 8))
        self.assertFalse(retry.sample(3, tone, False, 9))
        tone[0]["packets"] += 10
        self.assertTrue(retry.sample(3, tone, False, 11))

    def test_camera_requires_one_visible_changing_remote_track_and_subscription(self):
        camera = Evidence("camera-live", False)

        def sample(t, luma, frames, track=1, subscribed=True, flags=5):
            return camera.sample(flags, [{"id": track, "luma": luma, "frames": frames, "decoder": False}], subscribed, t)

        self.assertFalse(sample(0, 15, 0))
        self.assertFalse(sample(2, 15, 20))  # frozen picture, growing decodes
        self.assertFalse(sample(3, None, 21))
        self.assertFalse(sample(4, 15, 22, subscribed=False))
        self.assertFalse(sample(5, 15, 23, flags=7))
        self.assertFalse(sample(6, 15, 24))
        self.assertFalse(sample(7, 240, 32))
        self.assertFalse(sample(8, 15, 44, track=2))  # a replacement starts a new window
        self.assertFalse(sample(9, 240, 54, track=2))
        self.assertTrue(sample(10, 15, 69, track=2))

    def test_live_track_requires_observed_transform_to_match_each_requested_mode(self):
        for phase, flags in (("mic-live", 3), ("mic-retried", 3), ("camera-live", 5)):
            for required in (True, False):
                for observed in (True, False, None):
                    with self.subTest(phase=phase, required=required, observed=observed):
                        evidence = Evidence(phase, required)
                        for t in range(3):
                            row = {"id": 1, "decoder": observed, "tone": True, "packets": t * 5,
                                   "frames": t * 5, "luma": 240 if t == 1 else 10}
                            if observed is not required:
                                with self.assertRaises(ProbeError):
                                    evidence.sample(flags, [row], True, t)
                                break
                            self.assertEqual(evidence.sample(flags, [row], True, t), t == 2)

    def test_matrix_keeps_each_labs_credentials_and_continues_after_a_failed_case(self):
        labs = [Lab("http://127.0.0.1:8001", Path("off.json"), "disabled"),
                Lab("http://127.0.0.1:8002", Path("on.json"), "required")]
        args = SimpleNamespace(scenario="both", executable=Path("talk-desktop"))
        report = {"scenarios": {}}

        async def exercise(executable, lab, browser, scenario, result):
            if scenario == "camera" and not lab.encrypted:
                raise ProbeError("fixture failure")
            result["stage"] = "complete"

        with patch("macos_device_call.scenario_run", side_effect=exercise) as run_case:
            asyncio.run(run_cases(args, labs, None, report))
        self.assertEqual([(call.args[1], call.args[3]) for call in run_case.await_args_list],
                         [(lab, scenario) for lab in labs for scenario in ("microphone", "camera")])
        self.assertEqual(report["scenarios"]["camera-disabled"]["failure"], "fixture failure")
        self.assertEqual(report["scenarios"]["camera-required"]["stage"], "complete")

    def test_offer_and_answer_correlate_to_native_publisher_and_negotiation(self):
        observer = Subscription()
        observer.observe(json.dumps({"type": "event", "event": {"type": "join", "target": "room",
                         "join": [{"sessionid": "native", "roomsessionid": "room-session"}]}}), False)

        def message(outgoing, kind, peer, sid, room_type="video"):
            side = "recipient" if outgoing else "sender"
            observer.observe(json.dumps({"type": "message", "message": {
                side: {"type": "session", "sessionid": peer}, "data": {"type": kind, "sid": sid, "roomType": room_type}
            }}), outgoing)

        message(False, "offer", "native", "s1")
        message(True, "answer", "bob", "s1")
        message(True, "answer", "native", "s2")
        message(True, "answer", "native", "s1", "screen")
        self.assertFalse(observer.subscribed("room-session"))
        message(True, "answer", "native", "s1")
        self.assertTrue(observer.subscribed("room-session"))
        self.assertFalse(observer.requested("room-session"))  # publisher-triggered offer
        message(True, "requestoffer", "native", None)
        self.assertTrue(observer.requested("room-session"))
        self.assertFalse(observer.subscribed("other-session"))


if __name__ == "__main__":
    unittest.main()
