"""The lab observer records mutation ordering without copying private bytes."""
import time
import unittest

from signaling_metadata import Requests


class RequestMetadataTests(unittest.TestCase):
    def observer(self):
        rows = []
        return Requests(1, rows, time.monotonic()), rows

    def test_fragmented_headers_and_bodies_keep_only_allowlisted_metadata(self):
        observer, rows = self.observer()
        body = b"flags=7&private=never-record-this"
        request = (
            b"PUT /ocs/v2.php/apps/spreed/api/v4/call/private123?secret=hidden HTTP/1.1\r\n"
            b"Authorization: Basic private-credential\r\nUser-Agent: Chrome/fixture\r\n"
            + b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n" + body
            + b"POST /ocs/v2.php/apps/spreed/api/v4/room/private123/participants/active HTTP/1.1\r\n\r\n"
        )
        for byte in request:
            observer.feed(bytes([byte]))
        self.assertEqual([(r["method"], r["endpoint"], r["role"]) for r in rows],
                         [("PUT", "call", "browser"), ("POST", "room-active", "native")])
        for private in ("private123", "private-credential", "never-record-this", "hidden"):
            self.assertNotIn(private, str(rows))
        self.assertEqual(observer.buf, b"")

    def test_pipelined_requests_skip_unrelated_routes_and_body(self):
        observer, rows = self.observer()
        body = b"POST /ocs/v2.php/apps/spreed/api/v4/call/fake HTTP/1.1\r\n\r\n"
        observer.feed(b"POST /login HTTP/1.1\r\nContent-Length: "
                      + str(len(body)).encode() + b"\r\n\r\n" + body
                      + b"POST /ocs/v2.php/apps/spreed/api/v4/call/testroom HTTP/1.1\r\n\r\n")
        self.assertEqual(len(rows), 1)

    def test_unsupported_or_oversized_framing_stops_observation(self):
        for header in (b"X" * 16385, b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
                       b"POST / HTTP/1.1\r\nContent-Length: -1\r\n\r\n"):
            observer, rows = self.observer()
            observer.feed(header)
            observer.feed(b"POST /ocs/v2.php/apps/spreed/api/v4/call/testroom HTTP/1.1\r\n\r\n")
            self.assertTrue(observer.stopped)
            self.assertEqual(rows, [])

    def test_transcript_capacity_is_shared_with_websocket_observer(self):
        observer, rows = self.observer()
        rows.extend({} for _ in range(1600))
        observer.feed(b"PUT /ocs/v2.php/apps/spreed/api/v4/call/testroom HTTP/1.1\r\n\r\n")
        self.assertEqual(len(rows), 1600)


if __name__ == "__main__":
    unittest.main()
