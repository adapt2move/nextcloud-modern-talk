"""Regression checks for interpreting real outage timelines, not media qualification."""
import unittest
from recovery_checks import native_recovery


def row(ms, phase, audio=0, video=0, screen=False, generation=1):
    return dict(ms=ms, phase=phase, audio_received=audio, video_received=video,
                screen_flowing=screen, generation=generation)


class RecoveryChecks(unittest.TestCase):
    def test_later_share_stop_cannot_erase_observed_outage_recovery(self):
        timeline = [row(1000, "Active", 100, 20),
                    row(2000, "Reconnecting", 150, 25),
                    row(5000, "Active", 700, 100, True),
                    row(8000, "Reconnecting", 900, 150, True),
                    row(9000, "Active", 300, 40)]  # Stopped screen counters retire.
        self.assertEqual(native_recovery(timeline, 1500),
                         dict(reconnecting_observed=True, media_resumed=True,
                              screen_flow_after_recovery=True))

    def test_prefault_activity_does_not_prove_recovery(self):
        timeline = [row(1000, "Active", 1000, 100, True),
                    row(2000, "Reconnecting", 1000, 100),
                    row(5000, "Reconnecting", 1300, 100)]
        checks = native_recovery(timeline, 1500)
        self.assertTrue(checks["reconnecting_observed"])
        self.assertFalse(checks["media_resumed"])
        self.assertFalse(checks["screen_flow_after_recovery"])

    def test_initial_reconnecting_before_injection_is_not_the_outage(self):
        timeline = [row(0, "Reconnecting"), row(4000, "Active", 1000, 100, True)]
        self.assertFalse(any(native_recovery(timeline, 2000).values()))

    def test_new_call_generation_uses_its_own_media_counters(self):
        timeline = [row(2000, "Reconnecting", 2000, 200),
                    row(5000, "Active", 150, 20, generation=2)]
        self.assertTrue(native_recovery(timeline, 1500)["media_resumed"])

    def test_audio_alone_or_a_stale_screen_is_insufficient(self):
        timeline = [row(2000, "Reconnecting", 100, 10, True),
                    row(5000, "Active", 800, 10)]
        checks = native_recovery(timeline, 1500)
        self.assertFalse(checks["media_resumed"])
        self.assertFalse(checks["screen_flow_after_recovery"])


if __name__ == "__main__":
    unittest.main()
