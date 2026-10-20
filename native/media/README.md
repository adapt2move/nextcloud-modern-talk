# Native helper, IPC version 2

This is project-owned C++ embedding of the exact ADR-005 WebRTC source. The only
implemented runtime entry point is `--diagnostic-synthetic WATCHDOG_SOCKET`.
`--version` identifies the source pin and synthetic-only status. There is no
production native device or call-authority entry point. The Rust media adapter
continues to report `Unconfigured` to the app.

`tools/media/fetch.py` checks immutable source revisions and runs DEPS-pinned
hooks outside the repository. `build.py` applies the deterministic owned patch,
builds with the fetched GN/Clang/Ninja tools, and records hashes. The native
engine's own DEPS Rust compiler is distinct from project Rust1.99.1 and appears
in the build record. Do not replace either source pin or compiler silently.

The helper owns one PeerConnection, one local synthetic 640×360/20fps camera,
one 48kHz mono sine source and bounded-count decoded sinks. These are actual
native VP8/Opus encode/decode and DTLS/SRTP paths, with no WebView or third-party
Rust WebRTC wrapper. The synthetic profile is deliberately smaller than the
accepted 720p30 performance workload. Native capture, AVFoundation,
ScreenCaptureKit, macOS audio/permissions and GPUI shared surfaces are absent;
they are not hidden behind successful stub acknowledgements.

## Framing

Integers are big endian. A 4-byte length precedes each ordinary record and counts
only its body. Check `88 <= length <= 8192` **before reading or allocating the
body**. The 88-byte body header is:

| Bytes | Meaning |
| --- | --- |
| 0..4 | ASCII `NMTM` |
| 4..6 | Version 2 (`u16`) |
| 6..8 | Kind (`u16`) |
| 8..72 | Four random local 16-byte IDs: account, host, runtime, call |
| 72..80 | Nonzero operation (`u64`), nondecreasing; one SDP's chunks share it |
| 80..88 | Nonzero strictly increasing serial (`u64`) |

No server session/room ID, app credential, native pointer, frame or snapshot is
serialized. Scope is fixed at creation; obsolete/reordered frames fail closed.
Commands are Create1 (one byte explicit synthetic microphone/camera intent),
Offer2, Description3, Candidate4, Stop5, Stats6 and diagnostic executor stall7.
Events are Ready100, LocalDescription101, LocalCandidate102, Connected103,
Counters104, Drained105 and Failed106 (one allowlisted numeric category).
Unknown commands/versions terminate admission. At most512 commands and64 local
or remote ICE candidates are accepted during one30-second absolute synthetic diagnostic lifetime.

SDP uses one lazy64KiB assembly slot: `total:u32, offset:u32, kind:u8` (offer1 /
answer2), then1..8095 bytes. Offsets must be contiguous and totals, operation and
kind identical; final data starts with `v=0\r\n`, has no NUL, and lines <=2048
bytes. Initial negotiation and at most one additional description exchange are admitted; the same transport identity and m-sections must remain. ICE restart, added tracks and unsolicited glare are unsupported. ICE is `mid_length:u8,
mline_index:u8, mid bytes, candidate bytes`, with mid1..64, candidate<=2048 and
bounded index. SDP and ICE Debug output is redacted; helper stderr is discarded.

There is at most one audio and one video section and one application section.
The I2 helper builds dcSCTP and admits reliable ordered DCEP channels with empty
protocol and only `status` / `simplewebrtc` labels, at most two over its entire
lifetime. It creates local `status` when offering; an answerer admits the two
Talk browser channels. This is private protocol cooperation, with no generic
application messaging or renderer API. Extra AV sections, RID/simulcast and
unknown media kinds fail before native SDP parsing. The owned factories only
instantiate VP8/Opus from the offered intersection.

The SCTP preflight and allocation hooks are in `sctp_limits.*` and the exact
upstream patch in `patches/sctp.py`. Admission limits are packet8192 bytes,
16 chunks, ordered DATA only, stream ID below4, two channels, DCEP24 bytes,
message4096 bytes and16 fragments/message. A fixed64-fragment ledger charges
fragments plus complete messages to64KiB, and complete messages to16 slots.
Pre-observer receive storage is16 messages/64KiB; send storage64KiB total and
32KiB/stream, with16 sends/64KiB over the helper lifetime. Callback deferral charges at most64 pending/executing callbacks and64KiB of
retained payload across reentrant dispatch, before emplacing each callback.
Charges release only after the callback returns; dispatch depth is at most2.
At most three vector stores (two executing and one pending) can retain capacity
for64 fixed entries each; bounded reason strings and reset-stream vectors have
separate capacity overhead. This corrects the earlier per-vector count check. TSN distance128 bounds gap-range growth. Unsupported unordered,
partial-reliability, interleaving, binary or stream-addition profiles terminate.
Native receive callbacks count at most2048 messages and retain no message data.
These hooks require independent allocation/security review; they do not prove
all WebRTC/DTLS/task/container allocations fit the accepted whole-media budget.

Counters104 contains eighteen `u64`: synthetic video produced, video decoded,
audio10ms produced, nonzero decoded audio10ms, successful native socket sends,
denied socket sends, last send time, rejected decode frames, elapsed time, first
revoke time, channels created/opened/closed, data messages/bytes received, data
rejections, completed negotiations and finite signaling state. Times are
helper-relative microseconds. Failed106 contains an allowlisted numeric reason;
SCTP admission reasons identify a finite frontier. Neither raw SDP nor channel
messages are logged. These counters are not physical-device attestations.

## Priority lifetime and shutdown

Safe Rust owns a two-entry application registry, with at most one live/retiring
slot per account. A retiring helper retains the slot until child exit and both
bounded IPC workers settle. Command and event channels each hold at most16
records; overload revokes instead of growing a queue. A fixed eight-entry process-lifetime tombstone ledger rejects reuse of an
exact synthetic scope, including failed startup. Exhaustion returns Busy;
there is no implicit ledger reset or helper recreation for the same call.
No GUI callback is allowed
to execute blocking startup/process I/O. Preview reuse is a future same-slot
transition; no separate preview process/path exists in this increment.

A private0700 directory and Unix stream carry fixed80-byte watchdog frames:
`NMW1`, the64-byte scope, TTL milliseconds (`u32`,1..100), increasing serial
(`u64`). Rust sends100ms renewals every25ms on an independent worker; these are
synthetic diagnostic keepalives, **not production policy grants**. EOF, wrong
scope, stale serial, expiry,30-second absolute lifetime or revoke cannot renew
an old generation. The Rust absolute start is recorded before process startup;
the helper also enforces30 seconds from its own birth independently of Rust.
Renewal, negotiation and errors never slide either deadline. This extended
observer window is PM-authorized I2-MEDIA-DIAGNOSTIC-WINDOW-01, not sustained
production qualification. Original15-second results retain their attribution.
Native watchdog and Rust child owner each escalate after
200ms; neither depends on ordinary IPC, remote leave or signaling completion.

The owned upstream patch checks admission immediately before
`PhysicalSocket::Send/SendTo`, including retransmissions on that backend, and
counts successful writes. Already submitted kernel packets cannot be recalled;
the send check/syscall boundary still needs broader concurrent and platform
qualification. Stop first closes admission, stops/joins synthetic production,
detaches sinks, closes PeerConnection, releases factory/ADM, stops native
threads, and only then emits Drained. An escalated exit is
`TerminatedUnconfirmed`, never a device-release acknowledgement. Clock/suspend
semantics are not qualified for production leases; no recovery or auto-rejoin
path exists.

See [allocation coverage](../../docs/architecture/media-allocation.md) and the
[development guide](../../docs/development.md) for execution constraints.
