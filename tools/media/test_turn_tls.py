"""Lifetime guards for the Linux TURN fixture runner; no server or network I/O."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import turn_tls


@unittest.skipUnless(sys.platform == "linux", "Linux network-namespace runner")
class NamespaceOwnership(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.private = Path(self.temporary.name)
        factory = mock.patch.object(turn_tls.subprocess, "Popen")
        self.process = factory.start()
        self.addCleanup(factory.stop)
        self.child = self.process.return_value.__enter__.return_value
        self.child.pid = 12345
        self.child.wait.return_value = 0
        signals = mock.patch.object(turn_tls.os, "killpg")
        self.signals = signals.start()
        self.addCleanup(signals.stop)

    def assert_group_closed(self):
        self.signals.assert_has_calls([
            mock.call(12345, signal.SIGTERM), mock.call(12345, signal.SIGKILL)])
        self.assertTrue(self.process.call_args.kwargs["start_new_session"])
        self.assertEqual(self.process.call_args.args[0][:3], ["unshare", "--net", "--"])

    def test_success_reads_only_the_owned_result_and_reaps_the_group(self):
        expected = {"result": "pass"}
        (self.private / "result.json").write_text(json.dumps(expected))
        self.assertEqual(turn_tls.in_namespace(self.private), expected)
        self.assert_group_closed()

    def test_timeout_closes_the_server_and_test_process_group(self):
        self.child.wait.side_effect = [subprocess.TimeoutExpired("fixture", 240), 0, 0]
        with self.assertRaises(subprocess.TimeoutExpired):
            turn_tls.in_namespace(self.private)
        self.assert_group_closed()

    def test_failed_child_cannot_be_reported_as_success(self):
        self.child.wait.return_value = 1
        with self.assertRaises(subprocess.CalledProcessError):
            turn_tls.in_namespace(self.private)
        self.assert_group_closed()

    def test_inner_mode_rejects_the_parent_namespace_before_network_changes(self):
        parent = str(os.stat("/proc/self/ns/net").st_ino)
        with mock.patch.object(argparse.ArgumentParser, "parse_args", return_value=argparse.Namespace(inside=True)), \
                mock.patch.dict(os.environ, NMT_TURN_PARENT_NETNS=parent, NMT_TURN_PRIVATE=str(self.private)), \
                mock.patch.object(turn_tls, "inside") as inside:
            with self.assertRaises(SystemExit) as failure:
                turn_tls.main()
            self.assertEqual(failure.exception.code, 2)
            inside.assert_not_called()


if __name__ == "__main__":
    unittest.main()
