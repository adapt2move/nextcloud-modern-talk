"""Content-free relay regressions; no Docker or external service required."""
import asyncio
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

from resilience import FaultRelay


class Writer:
    def __init__(self):
        self.bytes = bytearray()
        self.closed = False

    def write(self, data):
        self.bytes.extend(data)

    async def drain(self):
        pass

    def close(self):
        self.closed = True

    async def wait_closed(self):
        pass


class Faults(unittest.IsolatedAsyncioTestCase):
    async def request(self, mode, query=b"", ordinal=0):
        with tempfile.TemporaryDirectory() as directory:
            control = Path(directory) / "mode"
            control.write_text(mode)
            relay = FaultRelay("10.1.2.3", 47407, control)
            relay.requests = ordinal
            relay.chat_reads = ordinal
            reader = asyncio.StreamReader()
            reader.feed_data(b"GET /ocs/v2.php/apps/spreed/api/v1/chat/fixture" + query + b" HTTP/1.1\r\nHost: fixture\r\n\r\n")
            reader.feed_eof()
            writer = Writer()
            remote = Writer()
            upstream = asyncio.StreamReader()
            upstream.feed_data(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            upstream.feed_eof()
            with patch("resilience.asyncio.open_connection", AsyncMock(return_value=(upstream, remote))) as opened, patch("resilience.asyncio.sleep", AsyncMock()) as delay:
                await relay.connection(reader, writer)
            self.assertTrue(writer.closed)
            self.assertFalse(relay.tasks)
            return relay, writer, opened.call_count, [call.args[0] for call in delay.call_args_list]

    async def test_latency_applies_to_every_request_and_poll_delay_is_scoped(self):
        for mode, seconds in (("latency-1500", 1.5), ("latency-4000", 4)):
            relay, _, opened, delays = await self.request(mode)
            self.assertEqual((opened, delays, relay.injected[mode]), (1, [seconds], 1))
        self.assertEqual((await self.request("slow-poll"))[3], [])
        self.assertEqual((await self.request("slow-poll", b"?lookIntoFuture=1"))[3], [4])

    async def test_gateway_and_drop_cycle_never_forwards_faulted_reads(self):
        for ordinal, status in ((2, b"504"), (5, None), (8, b"502")):
            _, writer, opened, _ = await self.request("intermittent", ordinal=ordinal)
            self.assertEqual(opened, 0)
            if status:
                self.assertIn(b"HTTP/1.1 " + status, writer.bytes)
            else:
                self.assertEqual(writer.bytes, b"")
        self.assertEqual((await self.request("intermittent", ordinal=0))[2], 1)


if __name__ == "__main__":
    unittest.main()
