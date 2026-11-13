# Architecture

NextCloud Modern Talk is a Rust workspace with native GPUI presentation. The
workspace manifests, `Cargo.lock` and `rust-toolchain.toml` are the build inputs;
[development](../development.md) covers prerequisites and checks.

## Module boundaries

| Owner | Responsibility |
| --- | --- |
| `nextcloud-talk-api` | Origin-bound HTTP/OCS, typed protocol values, signaling and file transport |
| `talk-core` | Domain policy, account/conversation/call identities, bounded UI snapshots and typed intents |
| `talk-adapters` | Credential providers, persistence transactions, network integration, updates and media engine |
| `apps/desktop` | Application composition, native devices, settings, GPUI windows and presentation |
| `apps/bootstrap` | Headless bootstrap and contract checks |

A consequential resource has one owner with a small interface. Extend that owner
before adding facade layers, event buses or feature-specific crates. Views do no
network, database, credential or media I/O. The API crate has no application-policy
or native-device dependency. Rust application code forbids unsafe code; native FFI
stays in reviewed dependencies.

[Architecture checks](../../tools/check_architecture.py) enforce dependency
boundaries, feature selection, source provenance and presentation restrictions.
[Boundary fixtures](../../tests/contracts/README.md) test intended rejection paths.
These lexical checks supplement compilation and behavioral tests; they are not a sandbox.

## Native UI

GPUI owns rendering, focus and window lifetimes. Composition injects bounded
snapshots and typed intents/results into presentation. No embedded WebView is used;
Login Flow v2 opens the system browser. Collections are paged or virtualized, and
input entities own cancellable caret tasks. Frames never enter chat serialization.

The local GPUI patch and bundled assets retain exact provenance in
[UPSTREAM.json](../../vendor/gpui/UPSTREAM.json) and [NOTICE.md](../../NOTICE.md).
See [design](../design/README.md) for UI rules. Compilation does not establish
accessibility, IME, device or permission support on another platform.

## Accounts and storage

Every operation carries its account/runtime generation and narrower conversation
or call scope. Retired completions cannot restore authority or update a new scope.
The account owner coordinates login, restoration, cancellation, logout and cleanup.
Normal sign-in is durable: prefer the native credential store, with an explicitly
selected **unencrypted private-file** fallback when native storage is unavailable.
There is no silent provider migration or master-password setup.

Use macOS Keychain, Windows Credential Manager or Linux Secret Service through the
platform adapter. Private files require verified per-user ownership and restrictive
POSIX permissions or Windows DACLs. The committed ledger names the exact eligible
provider/item; enumeration and a successful write alone cannot authorize restoration.
Retain the previous committed credential until replacement is confirmed. Late or
uncertain writes require settlement, and deletion failure remains visible. Logout
suppresses restoration account-wide, including residual items in another provider.

The detailed [identity contract](identity-contract.md), [account requirements](../product/accounts.md)
and [security requirements](../quality/security.md) own transaction, crash recovery,
file-access and cancellation invariants. Implementation rationale belongs beside code.

## Transport

`nextcloud-talk-api` owns bounded HTTP/TLS/OCS requests, canonical origins, response
limits, timeouts and typed errors. Credentials are confined to the authorized origin;
redirects, downloads and URLs supplied by servers cannot broaden that authority.
Errors preserve safe context without credentials or message contents.

Adapters map protocol observations to application policy. Mutations with uncertain
outcomes use bounded read-only reconciliation instead of blind retries. A room-level
denial, timeout or rate limit is not account revocation. The [integration contract](nextcloud-contract.md)
defines admission, completion and shutdown; [protocol sources](../compatibility/baselines-and-sources.md#protocol-sources)
record the upstream references.

## Policy and lifetime

macOS authorization belongs to the running application's code identity. The
settings and notification owners use the vendored GPUI platform boundary for
AVFoundation, CoreGraphics and UserNotifications checks and explicit requests.
Native work is dispatched on the application's main queue, with one retained
callback admission per permission kind. A cancelled or timed-out waiter cannot
reclaim a still-live OS callback's slot or publish a late result. The Swift
delivery/PDF helper does not request authorization. Publication uses one persistent
Developer ID team; changing from an ad-hoc build can require a new OS approval.

Capabilities, configuration, current permissions, session state and explicit user
intent determine available actions. A server version or cached snapshot grants no
permission. Policy observations have finite freshness; unknown, restrictive or expired
state denies the affected operation. Hash-only responses cannot renew omitted policy.

Account rejection invalidates its generation, stops new authenticated work and closes
local media without waiting for server leave or credential cleanup. Room restrictions
act on their narrower scope. Resume revalidates authority and devices before use.
Every queue/cache/operation has finite capacity, lifetime, cancellation and overload
behavior. Preserve durable intents and required transitions when rejecting new work.

## Native media

The in-process `webrtc` engine and same-version `rtc` core own SDP, ICE, DTLS/SRTP
and SCTP. Safe GStreamer bindings provide codec and frame conversion; desktop device
owners use CPAL for audio and GStreamer for cameras. Account/call generations bind
signaling, publication grants, devices and frames. Leaving or losing authority closes
publication irreversibly; remote leave and device disposal are separate outcomes.

Selecting a conversation or ringing never opens devices. Explicit preview is separate
from call publication. Required E2EE and unsupported profiles fail closed. See
[known limitations](../limitations.md) for the actual supported profile.

Audio reception converts decoded Opus to mono S16LE at 48 kHz, then
`audiobuffersplit` forms exact 10 ms (480 sample) blocks. The appsink retains at
most twelve blocks (120 ms, enough for one maximum-duration Opus packet) and
drops the oldest under overload. Packet durations of
10/20/40/60 ms do not change the device contract. There is no second Rust PCM
splitter or intermediate PCM queue.

The migration to a connected `webrtcbin` pipeline is **incomplete**. Its
acceptance probes use the exact `gstreamer-webrtc` 0.25.4 bindings only in tests;
production still uses Rust WebRTC. Talk E2EE must seal whole encoded frames
before RTP fragmentation and open them after depayloading, retaining the clear
Opus TOC or VP8 10/3-byte header. GStreamer's `rtpvp8pay` parses the encrypted
VP8 partition header and rejects valid sealed frames. Moving encryption after
packetization would change Talk's frame protocol. The ignored
[`talk_e2ee_vp8_survives_gstreamer_payloader`](../../crates/talk-adapters/src/media/gstreamer_probe.rs)
acceptance test exposes this blocker with an unencrypted control. The test-only
`gst-plugin-rtp` 0.15.4 alternative (`rtpvp8pay2`) preserves sealed keyframes
and delta frames in the exercised pay/depay probe; it is not selected for
production. A compatible frame packetizer integration and
its packaged dependencies need qualification before replacing the shipping path.
The separate `webrtcbin_ice_restart_rotates_credentials` gate also rejects an
`ice-restart=true` offer that reuses both credentials. The pinned
[1.28.4 offer path](https://github.com/GStreamer/gstreamer/blob/1.28.4/subprojects/gst-plugins-bad/ext/webrtc/gstwebrtcbin.c#L3578)
still reuses previous credentials; replacing a peer instead would need a deliberate
signaling/lifetime design and recovery qualification. Key exchange, rotation, publication
gates and call/account generations remain with their existing owners.

The verified TURN TCP/TLS bridge remains required. The inspected
[libnice 0.1.23 RFC relay path](https://github.com/libnice/libnice/blob/0.1.23/agent/agent.c#L2913)
does not supply a TLS handshake or system certificate verification; its pseudo-SSL
branches are for older Google/MS compatibility modes. The macOS SDK recipe pins
0.1.23; Windows pins 0.1.22 with the same limitation. A `turns` URI alone is not
proof of equivalent TLS support. Each bridge owns a bounded task set for its
connections; EOF retires both stream halves, and call close aborts even an
incomplete TLS handshake instead of leaving detached reader/connection tasks.
A required TLS verifier that cannot initialize fails peer construction closed.
Native `webrtcbin`, ICE, DTLS/SRTP and RTP plugins
are prerequisites of the opt-in probes, not a shipping-backend selection.

The [legacy native helper](../../native/media/README.md) is diagnostic-only. Its
process isolation and allocation model do not apply to the shipping engine.
Application frame/queue bounds do not cap decoder, OS or GPU allocations. Complete
resource claims require the [measurement contract](measurement.md) and
[performance requirements](../quality/performance.md).
