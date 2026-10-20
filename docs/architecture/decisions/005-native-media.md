# ADR-005 — Maintained WebRTC engine and native device ownership

The client uses the maintained [`libwebrtc` Rust binding](https://docs.rs/libwebrtc/0.3.52/libwebrtc/) at **0.3.52**, with the matching native SDK pinned by [prepare_native.py](../../../tools/media/prepare_native.py). It does not require a LiveKit server: NextCloud Talk remains the account, conversation and signaling backend.

The binding owns the native WebRTC implementation of SDP, ICE, DTLS/SRTP, codecs and data channels. Project Rust remains `forbid(unsafe_code)`. Native FFI and C++ internals belong to the dependency; using a safe Rust API does not make those dependencies memory-safe or establish their resource limits.

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

Cargo and the native SDK are separate build inputs. The build helper verifies the SDK archive SHA-256 and the complete extracted file inventory before Cargo uses `LK_CUSTOM_WEBRTC`. Linux, macOS Apple Silicon and Windows MSVC have separate pins. The release package includes the SDK's complete `LICENSE.md` and the resolved Rust dependency notices. See [development prerequisites](../../development.md) and [release tooling](../../../tools/release/README.md).

The older [native helper](../../../native/media/README.md) remains a diagnostic/fault-injection harness. It is not the shipping call engine and its tests do not qualify the maintained binding. Its source/patch cache and historical allocation model are not build prerequisites for the app.

## Lifetime and supported profiles

A call carries its account runtime and call generation through signaling, policy, device handles and display frames. Restriction, expiration, logout or leave closes local publication before awaiting a server response. Old callbacks cannot reopen a retired call. Remote leave and native/device disposal are separate outcomes; the UI must preserve an unconfirmed cleanup error.

Current support and omissions belong in [current state](../../status.md). Required E2EE, external signaling, guest/federated profiles and screen capture are never inferred from a successful ordinary audio/video call. Unsupported paths remain unavailable until their complete native and backend flow exists. Screen sharing is unsupported in both directions: an incoming screen offer closes this ordinary call profile instead of producing a screen tile.

The alpha integrates the library in-process. It does **not** inherit process-crash isolation or allocator guarantees from the diagnostic helper. Application queue/frame bounds are implemented at the owning boundary; they do not hard-cap the engine's internal allocations, decoder buffers, OS drivers or GPU storage. The [allocation targets](../media-allocation.md) and [performance requirements](../../quality/performance.md) remain unqualified for the complete client. Do not describe this engine selection as a memory, energy or latency improvement without measurements.
