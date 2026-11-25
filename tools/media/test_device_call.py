"""Negative controls for the macOS device-call receiver verdict."""
import json
import unittest

from macos_device_call import Evidence, Subscription


class ReceiverTests(unittest.TestCase):
    def test_retry_needs_new_packets_and_the_test_tone_after_quiet(self):
        baseline = Evidence("mic-stalled")
        tone = [{"id": 1, "tone": True, "packets": 200}]
        self.assertFalse(baseline.sample(1, [], False, 0))
        self.assertFalse(baseline.sample(1, tone, False, 2))
        self.assertFalse(baseline.sample(1, [], False, 3))
        self.assertTrue(baseline.sample(1, [], False, 5))

        retry = Evidence("mic-retried")
        self.assertFalse(retry.sample(3, tone, False, 5))
        self.assertFalse(retry.sample(3, tone, False, 7))
        tone[0]["packets"] += 10
        self.assertFalse(retry.sample(1, tone, False, 8))
        self.assertFalse(retry.sample(3, tone, False, 9))
        tone[0]["packets"] += 10
        self.assertTrue(retry.sample(3, tone, False, 11))

    def test_camera_requires_one_visible_changing_remote_track_and_subscription(self):
        camera = Evidence("camera-live")

        def sample(t, luma, frames, track=1, subscribed=True, flags=5):
            return camera.sample(flags, [{"id": track, "luma": luma, "frames": frames}], subscribed, t)

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
