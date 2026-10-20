# ADR-007 — Origin-bound asynchronous Nextcloud transport

This contract defines bounded transport ownership within the three architectural layers in [ADR-002](002-deep-modules.md). The desktop composes identity and authenticated protocol operations; [current state](../../status.md) records supported scope and qualification limits.

## Decision

The independent [nextcloud-talk-api](../../../crates/nextcloud-talk-api/README.md) deep module owns HTTP/WSS, OCS/DAV decoding, session cookies and typed internal/HPB signaling. The [Nextcloud adapter](../../../crates/talk-adapters/src/nextcloud.rs) owns application authority, server-token/local-handle mapping, policy interpretation and operation reconciliation; identity and WebRTC/codec work remain in their adapter owners. Core receives typed bounded operation data; GPUI receives only the reviewed UI projection. Use pinned reqwest with Rustls platform verification, Tokio, serde/serde_json, url, zeroize, getrandom and cookie. [Cargo.lock](../../../Cargo.lock) fixes dependency resolution; preserve dependency licenses and provenance. GPUI's independent dependency closure shares the canonical lock. No generic bus, plugin framework or façade package is added.

Production origin admission accepts HTTPS only, normalizes the installation base and rejects credentials, query/fragment, control/backslash, dot traversal and ambiguous encoded paths. Private endpoint construction preserves exact origin and base path. Requests disable redirects, automatic retries, proxies and Referer; credentials never travel to a Location or federated host. TLS validation is not disabled. Only the default-off test seam admits literal 127.0.0.1 or ::1 HTTP; it rejects DNS aliases and other loopback addresses.

The module uses HTTP/1, at most 64 response headers, a three-second connect/request limit and the caller's earlier monotonic deadline. Every ordinary request checks cancellation; the atomic core primitive is observed every 25 ms only while work is in flight. Diagnostic signaling long poll has an explicit 35-second override, still capped by caller deadline. These values bound attempts; they are not measured performance or sleep-aware media guarantees.

## Bounds and failure behavior

| Owner/data | Limit and behavior |
| --- | --- |
| Shared NetworkRuntime | 8 account bindings, one per AccountId; 8 admitted HTTP futures; no waiting queue |
| Encoded request/response | 256 KiB; Content-Length checked before body read; every streamed chunk checked before accumulation; oversized body fails |
| Content decoding | Request identity encoding; gzip/Brotli/deflate/zstd disabled even if Cargo feature unification enables support; nonidentity response encoding rejected, no decompression allocation |
| JSON | Body bounded first; 4096 shared value/key/container budget and depth 64; insertion checked incrementally; duplicate keys and trailing data rejected |
| Cookies | At most 8, 4096 bytes per Set-Cookie, host-only and path-scoped; no Domain forwarding or persistence; credential values redacted/zeroizing |
| Room mapping/list | 256 private random handle bindings; full list capped at 100, no silent truncation; participants capped at 100 |
| History/messages | 50 per page; scoped cursor; text at most 131072 bytes; no rich parameter execution or automatic read marker |
| Send correlation | 100 retained entries, no automatic eviction/replay; random reference ID and own-actor match; ServerAccepted is not delivered/read |
| Diagnostic signaling | 16 remote peers; 32 signals per poll; SDP 65536 bytes/line 2048, ICE 2048, mid 64/index <24; privacy-redacted opaque handles |

Finite body/structure bounds do not mean every allocation equals the body size. HTTP/TLS framing, header parsing, serde string temporaries, native certificate checks and JSON/form escaping use dependency-owned buffers. Reqwest is restricted to HTTP/1 and the pinned Hyper buffer implementation; full process peaks/allocation instrumentation remain unqualified. The protocol crate does not own a content disk cache, desktop polling schedule or media frame queue.

Capabilities and effective room policy are freshly fetched before operations. Known 34/35 families plus required capabilities are necessary; version alone never permits an action. Unknown family, role/type/attributes/permission bits or missing policy stays unavailable. Classified/unsupported remote paths stay closed. Room membership is not call authority. The [core call state](../../../crates/talk-core/src/calls.rs) combines fresh policy, consent and native support before granting publication; an unsupported encryption requirement blocks the affected call.

Errors expose only an allowlisted kind/account/retry class, not response bodies, URLs or raw network errors. Explicit account probe rejection closes admission; action denial does not. 429 supplies a bounded retry delay but the adapter does not automatically retry. Dispatched mutations can return Uncertain. A bodyless 304 is successful exhaustion only for the dedicated history path; all redirect responses remain rejected.

## Scope and integration

The [desktop account composition](../../../apps/desktop/src/composition/accounts.rs) starts `AccountConnection` with the platform identity runtime and media service. The API exposes semantic chat, conversation, file, presence, notification and meeting operations, including typed HPB sessions. The adapter normalizes replies and checks application authority before projecting bounded results. Protocol coverage alone does not establish that every operation is available or verified in the desktop; the [feature inventory](../../compatibility/feature-inventory.md) records that scope.

The [identity/runtime contract](../nextcloud-contract.md) specifies exact construction, provider assumptions, unresolved-work retention and cancellation lifetime. Real-server integration and native storage/media qualification require separate evidence.
