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

The attachment composition owner retains one eight-file, thirty-minute selected-room
batch. It dispatches one upload/share at a time, consumes the caption only after
its first confirmed share, and preserves uploaded remote paths for definitive
share retries. Dispatched uncertainty pauses the batch without replay. Room or
account retirement cancels the work and fences late completions.

The same owner admits one native file-promise export against the exact captured
rich descriptor and current message policy. The existing transfer owner performs
staged non-overwriting publication. A bounded app-level scalar observation retains
only request, former context and publication outcome until the native foreground
waiter acknowledges it, including after navigation. This observation grants no
former account authority. Both sides have a 120-second deadline; an unacknowledged
composition outcome releases its slot after five further seconds. The presentation
waiter performs no I/O. AppKit owns its task and receives no authenticated route.
The vendored GPUI patch retains the weak delegate's source and releases the active
UI drag on native completion. Other platforms retain the default unsupported
outgoing-drag implementation. Download action buttons stop the ancestor drag admission on mouse-down; the file body remains draggable. Transfer progress is projected in the application title bar, outside the composer.

User-authored message links are separate from authenticated Cloud handoffs. Presentation emits the exact source and a bounded link ordinal. Composition resolves that ordinal through the pure text owner only after checking the current account, room, source, retention and exposure policy, then repeats those checks when draining its single two-second handoff slot. Only a canonical public HTTP/HTTPS value reaches the OS opener. Parsing or rendering never starts network or external-resource work.

## Attachment presentation and playback

The content adapter validates current room, attachment, retention and origin before resolving an original. Original raster/PDF reads and the separate bounded `VideoBytes` result never expose a remote route to presentation. Composition decodes raster images off the UI executor and owns their finite resident pixel budget: 64 MiB for one original and 96 MiB across admitted rasters. Preview generations distinguish closing and reopening the same file, in addition to account/runtime/room and exact descriptor checks.

Visible attachment requests precede additional avatars, with at most two requests per presentation tick and two admitted content jobs. Transient thumbnail failures allow at most three attempts in a 60-second window, with one- and two-second retry delays; at most 128 targets remain tracked. Room/account retirement cancels retries. Permission, authentication, missing-file and unsupported-format failures are not automatically retried. A permitted original used after a missing server thumbnail retains the existing 4 MiB, 8192-pixel and 16 Mi-pixel source limits, then shrinks to at most 320×240 before pixel retention or persistence.

One local video owner consumes finite in-memory bytes through explicit GStreamer demuxers and decoders. A single worker, one latest-frame slot and coalesced play/seek controls bound work; stop has priority. Frames move directly to composition-owned render images and never enter chat/event serialization or the durable content cache. Native teardown retains the single-player lease until the worker exits. Starting/seeking has a ten-second deadline; stop is reported unconfirmed after three seconds. Explicit process quit waits at most five seconds without fabricating resource-stop confirmation. The packaged native plugin/source closure remains owned by the release runtime definition.

The existing private account-data owner stores only reencoded, descriptor-bound thumbnails under [AUTH-009](../product/accounts.md#auth-009). Reads are length/dimension checked before decode and retirement fences late completions. Both owner hydration and an earlier UI load prefer the exact eligible local raster over a network fetch; the two-slot decode queue uses existing admission backpressure. Confirmed in-memory thumbnails follow their current descriptor/scope and capacity limits instead of a periodic one-minute lease. Failed required removals collapse to one retained account clear, retried on the existing owner tick with a five-second cooldown. Disposable previews yield to required history and outgoing work within the database ceiling. Cached presentation never grants network or attachment authority.

## Notifications

Account observation uses the authenticated, origin-bound NextCloud Notifications
API v2 list, with Spreed's structured notification identities. Advertised list
and user-status support, the intended user, `shouldNotify`, Do Not Disturb, current
room policy and bounded message/call context all gate delivery. Translated subjects
and notification action URLs cannot grant authority. History/context reads never
advance read markers or dismiss notifications. See the pinned
[Notifications API v2](https://github.com/nextcloud/notifications/blob/f106a3fedaaca27c54a5b701ad54f8b688e628ec/docs/ocs-endpoint-v2.md).

An empty HTTP 200 list confirms an available source. HTTP 204 means no registered
notification provider: clear source freshness and wait one hour before probing
again, as the official API requires. Transient failures clear freshness and back
off from five seconds to two minutes; explicit Retry-After is retained up to the
transport's five-minute ceiling. Admitted push hints coalesce into the existing
account schedule and cannot reset failures or bypass these cooldowns. A new runtime has a
fresh schedule; an older runtime cannot update it. Message and call channels remain
independent from room discovery and other accounts.

The desktop notification owner retains one worker, three priority slots and at
most 16 correlated completions. Queued ordinary alerts expire after five seconds;
incoming calls after 45 seconds. Foreground entry cancels queued/running ordinary
message alerts without dismissing a current call or an explicit local test. One
separate authorization slot uses the same worker. Checks have a three-second
deadline; an explicit first request has a two-minute deadline and retires queued
alerts before showing a sheet. Work waiting over five seconds cannot start a late
sheet. Stopped or superseded completions cannot change observed authorization.
Native helper cancellation kills and reaps its process; no extra idle timer is
introduced. These bounds do not establish native macOS delivery or permission UX.

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
