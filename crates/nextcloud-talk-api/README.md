# NextCloud Talk API

An unpublished Rust protocol owner extracted from the application's existing transport. It depends on HTTP/WebSocket TLS transport, JSON, URLs, cookies, zeroizing storage and Tokio. It has no dependency on `talk-core`, GPUI, credential providers, SQLite, devices or a WebRTC engine.

`Client` owns origin-bound authentication, bounded HTTP/OCS parsing, cookies, account leases, ordinary deadlines and no-replay write journals. `Request` and `Reply` describe semantic operations, without an arbitrary authenticated URL interface. Room tokens and server tag IDs are protocol values; application navigation IDs, permission decisions and UI projections stay in `talk-adapters`. The adapter temporarily projects typed observations into its existing conservative policy decoder, preserving unknown restrictions rather than replacing that policy during extraction.

An identity owner consumes `AppCredential` exactly once. It has no `Debug`, `Clone`, credential getter or serialization. Browser grants, durable eligibility, restoration, OS storage and logout remain owned by the existing identity runtime. A shared atomic cancellation signal closes both owners. `Request::Account` uses the pinned authoritative OCS rejection classifier and validates canonical identity before binding it; ordinary permission failures cannot revoke an account.

```rust,no_run
use nextcloud_talk_api::{AppCredential, Client, NetworkRuntime, Request, Reply, ServerOrigin};
use nextcloud_talk_api::scope::{BindingId, LeaseId, OriginId, ClockId, SessionScope, OperationContext, Cancellation, Moment};
use zeroize::Zeroizing;

async fn inspect(login_name: Zeroizing<String>, app_password: Zeroizing<String>)
    -> Result<(), nextcloud_talk_api::Error>
{
    let runtime = NetworkRuntime::new(ClockId::from_key(1));
    let scope = SessionScope { account: BindingId::from_key(1), runtime: LeaseId::from_key(1) };
    let origin = ServerOrigin::https("https://cloud.example.invalid/").unwrap();
    let credential = AppCredential::new(origin, login_name, app_password, None);
    let mut client = Client::new(credential, runtime.clone(), scope, OriginId::from_key(1), Cancellation::default())?;
    let context = OperationContext {
        scope, cancellation: Cancellation::default(),
        deadline: Moment { epoch: runtime.now().epoch, millis: runtime.now().millis + 3_000 },
    };
    if let Reply::Account(account) = client.request(&context, Request::Account).await? {
        assert!(!account.id.is_empty());
    }
    Ok(())
}
```

The identifiers above are caller-allocated equality-only lease keys. Production callers must allocate collision-resistant keys and keep the same runtime across accounts. They never derive authority from ordering generations. Responses and all transport queues have finite limits: eight accounts and request slots per runtime, 256 KiB JSON bodies, 4,096 JSON structural entries, 100 rooms/participants, 50 historical messages and a 100-entry send journal. Settled entries are reclaimed oldest-first; uncertain entries remain until reconciliation. Callers allocate fresh references and never rely on this finite recent-reference window for server idempotency. Duplicate JSON keys fail. Redirects, proxies, compression, TLS bypass and automatic retries are disabled. Ordinary operations have a three-second ceiling; signaling polls use a distinct 35-second ceiling and always honor an earlier caller deadline. Binary DAV transfers use a separate 32 MiB application limit, streamed 64 KiB writes and a 60-second ceiling, also limited by caller cancellation/deadline. PUT is create-only. The application stages downloads and publishes only completed files without overwriting an existing destination.

## Chat and conversations

`Request::Chat` adds finite future polling, rich/parent/reaction observations, replies and threads, edits/deletes, reactions, reminders, server schedules, message pins, mentions, shared objects and attachment probing/sharing. Room-scoped search uses the Nextcloud unified search provider. Hidden edit/delete/reaction events retain their canonical target; deleted text is discarded before projection. History/polling never implicitly marks read or clears notifications.

`Request::Conversation` covers metadata and emoji avatars, notification preferences, restrictions, lifecycle, membership and roles, directory invitations, bots/bans and bounded breakout basics. The application refreshes capabilities and room policy before dispatch. Provider translation/summaries and typing emission are explicitly unconfigured; unsupported/missing functionality is not reported as successful. The operation inventory remains partial rather than claiming a complete SDK.

`Client::poll` provides typed published-poll create/read/vote/close operations.
It preserves hidden results, vote limits and uncertain creation without automatic
replay. Closing retains the published poll; server draft management and export
are not part of this interface.

`Client::image` reads bounded authenticated user/room avatars and file previews
through known installation-relative routes. It accepts no arbitrary URL. Native
decoding, cache scope, access expiry and display belong to the application owner;
an available rich-object reference alone does not authorize downloading its file.

`Request::FindPeople` queries the server-filtered local-user directory without a room; `Request::CreateDirect` creates or returns a direct room. The application accepts only current, account-scoped directory selections and retains uncertain creation operations without replay. Group creation keeps its separate server permission.

`Client::own_user_status` reads the current user's account-correlated status through the existing bounded transport. Fresh chat-poll notification eligibility requires known non-silent incoming content, an explicit supported personal notification mode and known non-DND status. Omitted `silent` is decoded according to the pinned Talk response contract; malformed values remain unknown. Group defaults, broader mention types, thread subscription policy and unknown status remain suppressed until authoritative data is available.

`Client::poll_live_events` drains at most 32 account-scoped, coalesced HPB invalidations and reports the connection state. Internal-signaling deployments return `Internal`; application scheduling supplies bounded room/message reconciliation in either case. An event is a refresh hint, never media authority or proof that a mutation succeeded. The first transport failure is exposed only as a safe error category, without wire text or credentials.

## Meetings and WebRTC

Normal protocol operations include room activation/deactivation, call participants, join, flags, leave and signaling settings. `Client::open_meeting` consumes the successful normal join handle and returns account-bound reader/writer/stop owners with separate bounded signaling and policy lanes. Session, room, peer and negotiation identities are checked before delivery. Dropping a destructive poll or an uncertain send closes that meeting; `finish_meeting` admits another join only after the old local owner is gone and the server confirms the old call session absent. SDP/ICE have finite limits. Processing receipts do not prove peer delivery.

Engine-neutral `SessionDescription`, `IceCandidate` and validated STUN/TURN settings stay free of native dependencies. The application owns the Rust `webrtc` engine with GStreamer codecs, current policy leases, explicit capture intent and physical device lifetime. Incoming direct invitations come from authenticated notification rows plus current Talk notification state; they are account-bound and decline dismisses only that fetched invitation. API success alone never grants capture or native notification action authority.

The ordinary internal-signaling path was exercised against original Talk24.0.6/NC34.0.5 and Talk25.0.6/NC35.0.2: both SDP roles, bidirectional Opus/VP8/SCTP, incoming accept/decline, mute, explicit reconnect with devices off, local retirement on policy loss, and call-only leave while the other participant remains. These are finite synthetic-device backend observations, not physical-device or all-platform acceptance. The external-signaling profile uses one account-bound WebSocket owner with native-root TLS, finite resume/reconnect, scoped room membership and separate SFU publisher/subscriber routes. It was exercised with Talk 25.0.6, signaling 2.1.1 and Janus 1.3.0 for bidirectional media/status, explicit reconnect and causal policy revocation. UDP STUN gathering and forced UDP TURN relay media were also exercised. TURN/TCP, TURN/TLS, live credential rotation, federated/anonymous-guest calling, required E2EE, screen publication and conference-scale behavior remain unavailable or unqualified. The older default-off diagnostic bridge is separate from the normal account-owned meeting path.

## Protocol coverage and verification

[Operation inventory](spec/operations.json) classifies all 187 operations in the full Talk 25.0.6 specification, including relevant federation/avatar routes absent from the ordinary specification. [Provenance](spec/provenance.json) pins both Talk 24.0.6 and 25.0.6 full specifications, the ordinary v25 document, checksums and upstream AGPL license. Only metadata is retained; no generated HTTP wrapper or upstream implementation source is compiled. Application account/capability probes belong to Nextcloud OCS and are additional to that Talk inventory.

```sh
cargo test --locked -p nextcloud-talk-api
cargo clippy --locked -p nextcloud-talk-api --all-targets -- -D warnings
cargo clippy --locked -p nextcloud-talk-api --all-targets --features backend-test -- -D warnings
```

The API tests need no GPUI, OS keychain, SQLite or native media engine. Tests preserve the extracted transport's cookie, cancellation, deadline and uncertain-write behavior. Disposable-server and application regressions qualify only their named scopes.
