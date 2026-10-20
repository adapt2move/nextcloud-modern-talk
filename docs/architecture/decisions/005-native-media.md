# ADR-005 — Maintained WebRTC engine and native device ownership

The client uses the Rust [`webrtc` crate](https://docs.rs/webrtc/0.21.1/webrtc/) at **0.21.1**, the async runtime layer over the same-version `rtc` Sans-I/O protocol core. The registry archive SHA-256 is `b621e736302bef784948cc8d8099fd0a8fdb5757693c80a344ffbe38c49387d2`. NextCloud Talk remains the account, conversation and signaling backend.

`webrtc` owns SDP, ICE, DTLS/SRTP and SCTP data channels. Maintained GStreamer VP8/Opus elements own encoding, decoding, RTP receive reassembly, conversion and scaling, through the workspace's pinned safe Rust bindings. The engine exposes owned PCM/RGBA frames and engine-neutral configuration/state types. Project Rust remains `forbid(unsafe_code)`; native codec internals remain dependency code, and safe bindings alone do not establish their resource limits.

## Module boundaries

| Owner | Responsibility |
| --- | --- |
| `nextcloud-talk-api` | Origin-bound HTTP/OCS operations, meeting endpoints and bounded SDP/ICE values; no device or UI ownership |
| `talk-core::calls` | Scoped call state, user intent, policy/consent decisions and expiring publication grants |
| `talk-adapters::nextcloud` | Meeting admission, current participant sessions, signaling lanes, policy refresh and remote leave |
| `talk-adapters::media` | Maintained engine integration, bounded application handoff and irreversible call closure |
| Desktop devices | Selected CPAL microphone/output streams and GStreamer camera capture; actual disposal and failure reporting |
| Desktop composition | Connects account, call and device lifetimes; views receive typed state and explicitly scoped frames |

No device opens merely because a room is selected or an incoming call rings. Device preview is explicit and separate from call publication. Camera frames do not pass through conversation JSON or general UI snapshots.

## Source and distribution

[Cargo.lock](../../../Cargo.lock) fixes the Rust engine and codec bindings. GStreamer runtime and development libraries supply VP8/Opus codec elements as well as camera capture. The application requires no separate WebRTC SDK archive or custom compiler for that SDK. Packages carry resolved Rust notices and platform-native GStreamer licensing information. See [development prerequisites](../../development.md) and [release tooling](../../../tools/release/README.md).

The older [native helper](../../../native/media/README.md) remains a diagnostic/fault-injection harness. It is not the shipping call engine and its tests do not qualify the Rust engine. Its source/patch cache and historical allocation model are not build prerequisites for the app.

## Lifetime and supported profiles

A call carries its account runtime and call generation through signaling, policy, device handles and display frames. Restriction, expiration, logout or leave closes local publication before awaiting a server response. Old callbacks cannot reopen a retired call. Remote leave and native/device disposal are separate outcomes; the UI must preserve an unconfirmed cleanup error.

Current support and omissions belong in [current state](../../status.md). Required E2EE, external signaling, guest/federated profiles and screen capture are never inferred from a successful ordinary audio/video call. Unsupported paths remain unavailable until their complete native and backend flow exists. Screen sharing is unsupported in both directions: an incoming screen offer closes this ordinary call profile instead of producing a screen tile.

The alpha integrates the library in-process. It does **not** inherit process-crash isolation or allocator guarantees from the diagnostic helper. Application queue/frame bounds are implemented at the owning boundary; they do not hard-cap the engine's internal allocations, decoder buffers, OS drivers or GPU storage. The [allocation targets](../media-allocation.md) and [performance requirements](../../quality/performance.md) remain unqualified for the complete client. Do not describe this engine selection as a memory, energy or latency improvement without measurements.
