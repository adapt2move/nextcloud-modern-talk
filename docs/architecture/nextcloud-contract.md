# Talk protocol and application adapter

The independent [`nextcloud-talk-api`](../../crates/nextcloud-talk-api/README.md) crate is the deep protocol module. It owns HTTPS/OCS transport, typed Talk operations, bounded decoding, cookies, account leases and Talk signaling. It has no GPUI, database, credential-provider or native-media dependency.

[`talk_adapters::nextcloud`](../../crates/talk-adapters/src/nextcloud.rs) maps application scopes to protocol handles, applies capability/room policy and projects bounded results. [`AccountConnection`](identity-contract.md) supplies the admitted account lifetime. Desktop composition consumes `ProtocolRequest`/`ProtocolReply`; presentation never receives a network client.

## Authentication and scope

The identity owner consumes an app credential through an opaque intake. Login orchestration and durable storage do not move into the protocol crate. Validated origin, exact login name, canonical identity and cancellation stay private. The application uses collision-resistant account/runtime/operation IDs and rejects stale or wrong-scope results.

One shared network runtime bounds accounts and HTTP admissions. Request futures own their I/O; dropping one does not destroy the account or erase an uncertain mutation. An already dispatched write can have changed the server even when local observation is cancelled. Uncertain writes remain retained and use read-only reconciliation without automatic replay. Desktop composition owns the bounded optimistic intent queue; the identity worker owns its account-scoped persistence and dispatch markers under [AUTH-009](../product/accounts.md#auth-009). A local queued result is distinct from a protocol receipt. The transport never infers safe replay from an absent recent-history match or restores authority from cached content.

Origin/subpath validation, normal certificate trust, no authenticated redirects, typed routes and bounded response parsing apply before policy projection. The public interface does not expose arbitrary authenticated URLs. Ordinary requests, destructive signaling polls and binary transfers have distinct finite deadlines and still honor an earlier caller cancellation/deadline. Exact limits live in the crate's API documentation and tests.

## Policy and application results

Room IDs, recipient IDs, member IDs and call scopes are application-local handles. The adapter resolves only current bindings. Local/server defaults, unknown restrictions and role-specific permissions remain distinct. A capability name or successful REST response alone cannot authorize capture, server moderation or a different room.

Conversation and message writes return correlated results. Composition retains results until acknowledgement, refreshes the affected server projection and prevents a stale completion from repopulating another room. File transfers use bounded streaming, explicit server sharing and staged non-overwriting local downloads.

User-authored message links are separate from authenticated Cloud handoffs. Presentation emits the exact source and a bounded link ordinal. Composition resolves that ordinal through the pure text owner only after checking the current account, room, source, retention and exposure policy, then repeats those checks when draining its single two-second handoff slot. Only a canonical public HTTP/HTTPS value reaches the OS opener. Parsing or rendering never starts network or external-resource work.

## Attachment presentation and playback

The content adapter validates current room, attachment, retention and origin before resolving an original. Original raster/PDF reads and the separate bounded `VideoBytes` result never expose a remote route to presentation. Composition decodes raster images off the UI executor and owns their finite resident pixel budget: 64 MiB for one original and 96 MiB across admitted rasters. Preview generations distinguish closing and reopening the same file, in addition to account/runtime/room and exact descriptor checks.

One local video owner consumes finite in-memory bytes through explicit GStreamer demuxers and decoders. A single worker, one latest-frame slot and coalesced play/seek controls bound work; stop has priority. Frames move directly to composition-owned render images and never enter chat/event serialization or the durable content cache. Native teardown retains the single-player lease until the worker exits. Starting/seeking has a ten-second deadline; stop is reported unconfirmed after three seconds. Explicit process quit waits at most five seconds without fabricating resource-stop confirmation. The packaged native plugin/source closure remains owned by the release runtime definition.

The existing private account-data owner stores only reencoded, descriptor-bound thumbnails under [AUTH-009](../product/accounts.md#auth-009). Reads are length/dimension checked before decode and retirement fences late completions. Failed required removals collapse to one retained account clear, retried on the existing owner tick with a five-second cooldown. Disposable previews yield to required history and outgoing work within the database ceiling. Cached presentation never grants network or attachment authority.

## Calls

Calls use typed join/leave and participant observation, with internal signaling or the qualified external-signaling route for SDP/ICE. The separate [meeting owner](../../crates/talk-adapters/src/nextcloud/meeting.rs) refreshes the applicable policy, enforces lobby/consent/publication decisions and owns the call lifetime. The [media owner](../../crates/talk-adapters/src/media/service.rs) integrates the Rust `webrtc` engine, GStreamer VP8/Opus codecs and bounded application audio/video handoffs; device resources belong to the desktop device owner.

The protocol owner maintains one account-bound HPB WebSocket session when the server supplies usable external-signaling settings. TLS uses native roots; reconnect and resume remain finite. Publisher/subscriber routes correlate HPB and Talk session identities, while current REST membership and policy authorize media. Live events invalidate bounded application observations; they do not grant capture, expose credentials or replace authoritative reads.

The qualified profile uses Talk 25.0.6, signaling 2.1.1 and Janus 1.3.0. UDP STUN and TURN are supported by the pinned media engine; TCP/TLS TURN and live credential rotation remain unavailable. Guest/federated media, required call E2EE and screen publication are outside the profile. Unknown or unavailable required evidence leaves the action unavailable. A signaling receipt or local data-channel send is not proof of recipient delivery.

## Authoritative account rejection

<a id="i2-auth-rejection-classifier-01--authoritative-rejection"></a>

Account loss is confirmed only for the authoritative `GET ocs/v2.php/cloud/user` probe: complete bounded OCS JSON with status `failure`, integer code `997`, a string message and a present object/array data container, paired with HTTP `401`. The classifier never interprets or logs message text. Contradictory or malformed evidence remains ambiguous and cannot revoke the account. Ordinary room, action and signaling denials remain scoped failures.

The mapping follows the pinned [NextCloud 34 OCS response](https://github.com/nextcloud/server/blob/v34.0.5/lib/private/AppFramework/OCS/V2Response.php) and [NextCloud 35 current-user endpoint](https://github.com/nextcloud/server/blob/v35.0.2/apps/provisioning_api/lib/Controller/UsersController.php). A general HTTP 401 is not equivalent to this account-probe outcome. The identity owner performs local closure and durable suppression separately.

## Diagnostics and verification

The default-off `backend-test` diagnostic bridge remains a separate synthetic tool. Its duplex lanes retain bounded request/account admission, selected-session correlation, ordered cookie commits and a fixed first-terminal-cause record. Metadata survives lane/resource release without retaining credentials, responses or HTTP ownership. Diagnostic success does not promote an application call or a native device to supported status.

Use the [protocol crate tests](../../crates/nextcloud-talk-api/README.md#protocol-coverage-and-verification), [disposable backend lab](../backend-lab.md) and [development checks](../development.md). Protocol, synthetic media, ordinary desktop and platform-native results have different scopes. Historical per-candidate evidence is retained privately; public API documentation describes the current owner boundaries.
