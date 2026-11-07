"""Bounded HTTP mutation metadata for the disposable signaling observer."""
import re
import time


class Requests:
    """Observe request order only; skip bodies and never retain paths or headers."""

    def __init__(self, connection, trace, started):
        self.connection, self.trace, self.started = connection, trace, started
        self.buf, self.skip, self.stopped = bytearray(), 0, False

    def feed(self, data):
        if self.stopped:
            return
        while data:
            if self.skip:
                count = min(self.skip, len(data))
                self.skip -= count
                data = data[count:]
                continue
            end = data.find(b"\r\n\r\n")
            # Headers can cross read boundaries. No header value is copied to
            # the transcript; unsupported framing stops observation, not traffic.
            self.buf.extend(data[:end + 4] if end >= 0 else data)
            boundary = self.buf.find(b"\r\n\r\n")
            if boundary < 0:
                if len(self.buf) > 16384:
                    self.stopped = True
                    self.buf.clear()
                return
            header = bytes(self.buf[:boundary])
            remainder = bytes(self.buf[boundary + 4:])
            self.buf.clear()
            data = remainder + (data[end + 4:] if end >= 0 else b"")
            if len(header) > 16384:
                self.stopped = True
                return
            lines = header.split(b"\r\n")
            parts = lines[0].split(b" ")
            if len(parts) != 3:
                self.stopped = True
                return
            headers = {line.split(b":", 1)[0].lower(): line.split(b":", 1)[1].strip()
                       for line in lines[1:] if b":" in line}
            if b"transfer-encoding" in headers:
                self.stopped = True
                return
            try:
                self.skip = int(headers.get(b"content-length", b"0"))
            except ValueError:
                self.stopped = True
                return
            if not 0 <= self.skip <= 16 * 1024 * 1024:
                self.stopped = True
                return
            route = parts[1].split(b"?", 1)[0]
            endpoint = None
            if re.fullmatch(rb"/ocs/v2\.php/apps/spreed/api/v4/call/[a-z0-9]{4,30}", route):
                endpoint = "call"
            elif re.fullmatch(rb"/ocs/v2\.php/apps/spreed/api/v4/room/[a-z0-9]{4,30}/participants/active", route):
                endpoint = "room-active"
            method = parts[0].decode("ascii", errors="replace")
            if endpoint and method in ("POST", "PUT", "DELETE") and len(self.trace) < 1600:
                self.trace.append({
                    "ms": round((time.monotonic() - self.started) * 1000),
                    "connection": self.connection,
                    "role": "browser" if b"Chrome/" in headers.get(b"user-agent", b"") else "native",
                    "direction": "out", "type": "http", "method": method, "endpoint": endpoint,
                })
