# Security requirements

Status: target contract, not an audit or an implemented guarantee. Acceptance evidence belongs in the [verification process](verification.md). Source links were checked on 2026-10-18; implementation must recheck the selected platform and dependency versions.

## Q.SEC-01 — Threat model and boundaries

Protect app passwords, guest/session tokens, media keys, conversation content, attachments, account identity and device capture. Consider hostile conversation participants, malformed server responses, malicious links/files, an active network attacker, cross-account mistakes, stolen local files and accidental disclosure through logs or support bundles.

Trust boundaries are GPUI views → application services; services → credential/storage/media adapters; client → authenticated Nextcloud/signaling/TURN endpoints; and downloaded content → preview/OS handlers. The GPUI boundary is a software interface, not a browser sandbox or a separate security process. Rust does not eliminate risks in native media libraries or unsafe FFI.

The OS, its vault and the installed application are trusted components. A compromised OS/process, a recipient recording externally, and a malicious server with legitimate access to ordinary chat data are not prevented by this client. Call E2EE does not imply chat/file E2EE or anonymous metadata. Review findings must identify these limits rather than claim universal protection.

Acceptance: the architecture names owners for every boundary, identifies unsafe/native dependencies and traces each threat to a control or explicitly accepted limitation.

## Q.SEC-02 — Authentication and credential ownership

Use Nextcloud Login Flow v2 in the user's external browser. The client receives a dedicated app password; it does not collect or persist the user's main password or second factor. Bind the pending login, polling endpoint and returned account to the server the user selected. Validate schemes/origins before polling or following credential-bearing redirects; a changed server origin requires an explicit trust decision. Clear cancelled/expired login material. The distinction between login name and canonical user identity must survive authentication. [NC34 Login Flow](https://docs.nextcloud.com/server/34/developer_manual/client_apis/LoginFlow/index.html#login-flow-v2), [NC35 Login Flow](https://docs.nextcloud.com/server/35/developer_manual/client_apis/LoginFlow/index.html#login-flow-v2)

Only the authentication service and its transport/vault adapters may materialize credentials. GPUI view models, commands, events, background jobs and media-facing interfaces receive opaque account/session handles, authorized operations and redacted state. A handle is scoped to an account and runtime authorization generation; logout, replacement, account removal and confirmed account authorization loss invalidate it. It cannot be serialized into a reusable credential. Runtime generations, pending login attempts and vault-item generations have distinct identities; cancelling a replacement attempt must not invalidate an intentionally retained, still-valid older session. Explicitly injected test doubles must never be reachable as production vault fallbacks.

Acceptance: test cancellation and late login completion, account replacement and stale handles; inspect UI/event/diagnostic types for raw credentials. A new account cannot borrow another account's session, cache or authenticated connection.

## Q.SEC-03 — OS vault contract

| Platform | Required adapter target | Boundary to verify |
| --- | --- | --- |
| macOS, primary | Keychain Services; application-owned generic-password records | App access controls, signing/entitlement changes, locked state and user interaction |
| Windows, future | User-scoped Windows Credential Manager generic credentials | Correct user/logon scope and persistence; no machine-wide substitute |
| Linux, future | An available Secret Service implementation, or a separately reviewed native vault adapter | Provider presence, collection lock/prompt state and provider security properties |

These APIs have different security and availability semantics. Keychain supports access controls; Windows credential writes belong to the current user's credential set. The Secret Service specification does not make every provider equivalent, and lookup attributes are not a secret store. Provider qualification is required; an absent or unsuitable provider triggers the fallback below. [Apple Keychain Services](https://developer.apple.com/documentation/security/keychain-services), [Microsoft CredWriteW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew), [Secret Service attributes](https://specifications.freedesktop.org/secret-service/latest/lookup-attributes.html), [Secret Service limits](https://specifications.freedesktop.org/secret-service/latest/ch10.html)

The architecture must define asynchronous status, stage/activate replacement, resolve-for-use, reconcile and idempotent exact-generation delete operations. Results report the actual persistence mode, item generation and whether an operation has settled. A store acknowledgement is issued only after the adapter confirms success; metadata must not advertise durable login after an uncertain write. Serialize conflicting account operations. Stage a replacement in a distinct generation so an uncertain write cannot overwrite an older item that the user intends to retain. A platform that cannot provide this separation does not qualify for automatic replacement.

Record identity is application namespace + local profile ID + random account ID + secret kind + schema version + vault-item generation. The account record binds this identity to the canonical server base URL (including installation path) and server user identity. Store only nonsecret lookup metadata; never put tokens in record names/labels. Same-host installations, two users on one server and accounts on different servers must remain distinct. Do not wildcard-delete another profile's records. Disable credential synchronization/roaming by default where the selected API supports it; verify rather than assume this property.

### Restoration and uncertain writes

Before dispatching a vault write, durably record a nonsecret intent: account, attempted generation, previous retained generation (if any), pending operation and restoration disposition. Suppress automatic account restoration while the attempt is unresolved. Eligibility to restore comes from this explicit committed state **and** an accessible confirmed item; vault enumeration alone never grants eligibility. Missing, corrupt or incomplete metadata requires explicit recovery, not guessing from whatever items exist. The bootstrap must define atomic/durable metadata updates and crash ordering; if the pre-write intent cannot be committed, do not dispatch the write.

| Transition | Required outcome |
| --- | --- |
| Confirmed store, current attempt, user still requests persistence | Commit activation/restoration eligibility for that exact generation; only then show stored-login success. Retire an older item only through an authorized replacement/removal policy. |
| Timeout/lost completion | Keep the attempted generation quarantined and the account in unresolved-storage state. A timeout or a cancellation request is not proof that the provider did not write. |
| User retries storage | First reconcile the same operation and exact item identity/generation. Do not issue overlapping replacement writes or restore a cancelled generation. A new save intent needs a new generation when the old one was abandoned. |
| User chooses session-only | Commit account-level suppression covering **all** old/new restorable generations before activating the in-memory session. Abandon the pending store and clean up only its attempted generation. Keep an older retained item untouched and ineligible for automatic restoration. |
| User cancels authorization | Invalidate the attempt; discard its owned in-memory secrets and clean up only its attempted generation. With no prior session, remain unconnected. If the user deliberately retains a still-valid older persistent session, commit eligibility for that older generation alone; do not delete it or activate the replacement. |
| Late completion after fallback/cancel/replacement | It cannot activate a session or restore persistence. Reconcile/delete the abandoned generation, preserving the account's selected restoration disposition and any retained older item. |
| Process exits/crashes during any step | On restart, honor committed suppression and abandoned-generation markers before resolving credentials. An unresolved attempt cannot restore either generation automatically; offer explicit recovery/retention of an eligible older session or fresh login. A previously committed cancel-and-retain decision may restore only its named older generation. |

Reconciliation must establish both operation settlement (no pending writer can recreate the item) and exact-generation presence/absence. Absence observed before an outstanding write completes is insufficient. Retain a nonsecret cleanup marker and visible residual-storage warning until settlement and deletion/absence are confirmed; retry on an appropriate user/provider trigger without prompt loops. Cleanup authority covers the newly attempted generation, not unrelated or intentionally retained older credentials. A session-only account remains suppressed even after cleanup; only an explicit, confirmed save may enable persistent restoration again.

If the metadata update needed to suppress a previously persistent account cannot be committed, do not activate a replacement under a session-only promise. Explain the restart-safety failure and offer repair/retry or cancellation retaining the prior session. Local cancellation still invalidates the attempt. If pre-write intent was committed but a subsequent retention decision cannot be committed, restoration remains suppressed; otherwise the last committed disposition governs. This failure must never produce a secret-bearing recovery file.

Acceptance: native adapter tests cover create/read/replace/delete, two users, two installation paths, lock/unlock, denied access, signing changes and the [uncertain-write scenarios](verification.md#vauthstore--uncertain-storage-transitions). Fault injection must include crashes on both sides of each metadata/provider boundary. A mock vault alone does not qualify a platform.

## Q.SEC-04 — Structured errors and honest fallback

Every vault failure includes operation, provider, nonsecret account reference, category, retry guidance and a redacted native error code. At minimum:

| Category | Meaning and required response |
| --- | --- |
| `Missing` | No matching item. Request login; do not call it an incorrect password. |
| `Locked` | Evidence indicates a locked vault/collection. Offer an OS unlock action; no background prompt loop. |
| `AccessDenied` | OS authorization, entitlement or user access refusal. Explain storage access; preserve the record. |
| `Corrupt` | Record decoding/integrity/schema failure. Do not overwrite automatically; offer explicit repair/re-login. |
| `TemporarilyUnavailable` | Provider busy, disconnected or unavailable during this operation. Retry with bounds on an appropriate trigger. |
| `Unavailable` / `Unsupported` | No qualified provider or platform implementation. Offer session-only login. |
| `Cancelled` | User dismissed an OS prompt. Respect the decision; no immediate re-prompt. |
| `Other` | Unclassified failure. Preserve the native code without guessing at credentials or lock state. |

Mapping must be evidence-based. For example, Apple's `errSecInteractionNotAllowed` describes disallowed interaction, not a universal proof of a locked keychain; `errSecAuthFailed` is an OS authorization/authentication failure, not a Nextcloud credential verdict. [Apple interaction status](https://developer.apple.com/documentation/security/errsecinteractionnotallowed), [Apple authentication status](https://developer.apple.com/documentation/security/errsecauthfailed), [Secret Service locking](https://specifications.freedesktop.org/secret-service/latest/unlocking.html)

If a newly authorized app password cannot be stored, offer `Nur für diese Sitzung fortfahren` as the default continuation, alongside retry and cancel, as specified in [AUTH-004](../product/accounts.md#auth-004), subject to the restoration-suppression contract above. Cancel does not choose fallback for the user. For a known non-write or confirmed cleanup, explain: “Sichere Speicherung nicht verfügbar. Du bist nur für diese Sitzung angemeldet.” For an uncertain write, instead disclose: “Nur für diese Sitzung aktiv. Ein Speicherversuch ist ungeklärt; automatische Anmeldung ist gesperrt.” Keep the attempted-generation cleanup status visible, including after restart/cancellation, until reconciled. Provide retry/logout and, when supported, an OS unlock action. Do not report successful persistence, confirmed memory-only storage or wrong credentials when the result is uncertain. Do not silently switch back to persistent storage when the vault recovers; saving needs the stated user action. If an existing secret cannot be read, session-only use requires fresh authorization or an already valid in-memory session; it cannot recover an inaccessible secret.

Session-only secrets must not enter preferences, databases, caches, environment variables, command lines, crash attachments or an encrypted file with an adjacent key. Make no further vault write without an explicit save action. Minimize lifetime/copies and clear owned buffers on logout/exit where practical. This means no intentional new persistence by session-only mode; it cannot undo an already dispatched uncertain write or guarantee erasure from swap, OS crash capture, backups or a compromised process. Account-level suppression makes restart require login even when an older or uncertain item remains in the vault. Do not auto-delete an older vault item merely because its replacement failed.

Acceptance: fault-inject every category and uncertain store/delete outcomes. Check visible state, restart behavior and filesystem/log artifacts. Repeat the relevant cases on macOS with the actual Keychain adapter.

## Q.SEC-05 — Logout, revocation and failure lifecycle

| Event | Required behavior |
| --- | --- |
| Timeout, DNS failure, rate limit, server error, offline state | Preserve credentials and account data; reconnect with bounds. Never translate this into account deletion or password rejection. |
| Action permission denied or conversation/signaling/guest session expires | Restrict/recover the affected action or session after fresh checks. Do not infer account-wide credential revocation from a generic 401/403, room removal or signaling rejection. |
| Confirmed account authorization loss | Enter `reauthentication required` and execute the account shutdown contract below. Preserve permitted drafts and unrelated accounts. Do not assert a particular revocation cause without evidence. |
| Explicit logout | Stop requests/capture, invalidate handles, clear session secrets, remove this profile's stored credential and authenticated cache according to retention policy; keep only nonsecret account setup if the user chose to. |
| Logout/account removal while online | Attempt revocation of this client's app password, bounded in time. Remove local access even if the network request fails. Report whether server revocation was confirmed, not attempted or unconfirmed. |
| Logout/account removal while offline | Complete local logout; disclose “Lokal abgemeldet; Serverzugriff nicht bestätigt widerrufen.” Direct the user to server device/session settings. Do not persist a secret solely to retry revocation. |
| Vault deletion fails | Still invalidate the local account/session; record a nonsecret deletion-pending marker to prevent automatic reuse. Explain the residual vault item and offer retry/manual removal. Do not claim all credentials were deleted. |
| Remove account | Additionally remove account-scoped history/search indexes, thumbnails and temporary downloads; handle unsent drafts explicitly. User-exported files are outside app-cache deletion. |

### Confirmed account authorization loss

The authentication owner must classify evidence using endpoint/session scope and documented response semantics; the bootstrap must specify what confirms account rejection and how ambiguous errors are resolved. A still-working media connection is not proof of valid account authorization. Detection depends on a relevant authenticated observation; revocation on the server does not imply that the client learns of it instantly or that the server tears down an established media session.

Once account rejection is confirmed, invalidate its runtime authorization generation and close admission of authenticated operations before processing further successes. Cancel requests, transfers, queued sends, background work and signaling/reconnect work for that account; reject late login, request, policy and media callbacks carrying the old generation. Disable local publication immediately in the media admission path, stop owned microphone/camera/screen capture, discard queued unsent media and close the local call/session. The bootstrap must bind this transition to an owner and a finite, numeric capture/device-release deadline with an observable failure path. Local shutdown must not wait for a server leave, token revocation, vault operation or another network response. Already transmitted packets/requests cannot be recalled; do not promise remote deletion or undoing an accepted send.

Suppress automatic restoration/retry of the rejected account's stored generations, including any older stored credential. Apply the nonsecret restoration-state discipline above; if persisting suppression fails, local shutdown still completes and the UI discloses that suppression across restart is unconfirmed. Preserve allowed drafts and account identity; uncertain sends remain unsent/delivery-unknown until explicitly reconciled. Other accounts keep their sessions, drafts and activity. Show “Erneut anmelden”; never delete credentials or drafts solely because an action was forbidden or the network failed.

Resume only with validated new authorization for the intended account, a new runtime generation and fresh capability, room, consent and security/session checks. Reauthorization alone cannot replay queued sends or restart capture: retry/rejoin follows a deliberate user action and preserves mute/camera intent. An unrelated user's authorization cannot inherit the pending work.

The server supports deleting the app password in use; its documentation explicitly allows local account removal to continue after a failed server response. [NC34 app-password deletion](https://docs.nextcloud.com/server/34/developer_manual/client_apis/LoginFlow/index.html#deleting-an-app-password), [NC35 app-password deletion](https://docs.nextcloud.com/server/35/developer_manual/client_apis/LoginFlow/index.html#deleting-an-app-password)

Acceptance: exercise offline logout, server revocation, vault deletion failure, restart with a deletion-pending item and a late in-flight request. Apply [V.AUTH.LOSS](verification.md#vauthloss--confirmed-account-authorization-loss) during active media and a pending send; record generation invalidation, actual capture/publication stop and absence of automatic retry. Local deletion must not be described as remote revocation or secure erasure of backups.

## Q.SEC-06 — Redaction, caches and diagnostics

Use allowlisted diagnostic fields. Exclude authorization/cookie headers, app passwords, login/poll URLs and tokens, room/guest/share tokens, TURN credentials, SDP/ICE addresses, media keys and message/file contents by default. Sanitize URLs, errors, redirects, panic/debug formatting, tracing spans and HTTP bodies at collection time, before they reach logs. Hashing a low-entropy identifier is not sufficient anonymization. Reports must not contain private hostnames, user filesystem paths or unrelated account identities.

Cache only necessary account-scoped content with explicit bounds, retention and removal rules. Secrets never belong in the content cache. Stored chat/drafts and attachment previews remain sensitive user data even if filesystem permissions restrict access. At-rest protection claims require an implemented, reviewed key lifecycle; session-only mode must not create persistent protected-content keys or new durable content caches. On expiry, removal or access loss, invalidate content/search/preview entries as the product policy requires. Restrict notification previews for sensitive/restricted conversations; do not send conversation contents to telemetry by default.

Acceptance: seed synthetic canary secrets in all input channels, induce failures and inspect logs, caches, support bundles and screenshots. Diagnostics are opt-in and previewable. Raw memory profiles can contain secrets and must use synthetic lab accounts and redaction before publication.

## Q.SEC-07 — TLS, redirects and endpoint scope

Require authenticated TLS for production HTTP/WebSocket services; validate hostname and certificate chain using the declared trust policy. No “accept all certificates” or silent HTTPS downgrade. Support enterprise trust through a reviewed OS/custom-trust policy, not a global verification bypass. An explicitly labelled development-only loopback profile may use HTTP; it must not accept real accounts or become a general production exception.

Allow user-configured private Nextcloud hosts. Treat URLs received in content or server responses as untrusted: apply per-purpose origin/scheme policy, bind auth to the intended server/base path, drop credentials on cross-origin redirects, and reject credential-bearing URL userinfo. Federation, external signaling and downloads need distinct endpoint authorization; do not forward the Nextcloud app password just because a URL arrived from the server. Never let a message trigger arbitrary local-network fetches or OS protocol handlers.

Acceptance: test untrusted/expired/wrong-host certificates, redirects to a second origin, alternate installation paths, private servers, federated hosts and external signaling. Capture evidence that credentials never reach the unintended endpoint.

## Q.SEC-08 — Links, attachments and untrusted rendering

Render supported message formatting as native data, with no executable HTML/JavaScript or embedded WebView. Show the real destination before opening suspicious links; reject dangerous schemes and request explicit intent for supported external handlers. Previews must respect origin, privacy and size policies. Do not automatically fetch arbitrary remote images that expose the user's address.

Check attachment size, MIME/content mismatch, filenames, traversal, symlinks and destination overwrites. Stream downloads into restricted temporary storage; cancellation removes partial files. Bound image dimensions, decompression and preview work. Preserve OS quarantine/origin metadata where supported and require a user action before opening downloaded files. Do not claim that native rendering or an extension check makes arbitrary files safe.

Acceptance: malformed rich objects, deceptive links, oversized/corrupt images, archive bombs, traversal filenames, interrupted downloads and executable attachments cannot execute content, escape the chosen destination or exhaust unbounded resources.

## Q.SEC-09 — Native media and capture intent

The architecture must choose and pin a native WebRTC/media engine and define FFI ownership, callback cancellation, device lifetime and update responsibilities. Retain authenticated signaling and WebRTC transport protections; native integration must not disable peer/certificate/DTLS checks to achieve connectivity. TURN relays and ICE candidates have privacy implications that must be documented. [WebRTC security architecture, RFC 8827](https://www.rfc-editor.org/rfc/rfc8827.html)

Ringing alone must not start microphone, camera or screen capture or request their permissions. Accepting an incoming colleague's direct call enters the call flow directly, without the “Bereit, wenn du es bist.” device-prejoin interstitial. Necessary OS permission prompts, recording consent, lobby and security policy still apply before capture/publication. Acceptance is intent for the indicated call media, not permission to enable the camera or screen silently. The device-prejoin screen is for voluntary conference joining; any local preview there is explicit and unpublished until joining.

Request device permissions at the corresponding user action; deny/revoke/device-loss states must remain usable. Screen sharing needs an explicit source selection and a persistent capture indicator/stop action. On hangup, rejected join, permission loss, logout/removal, confirmed authorization loss of the call's account or teardown, stop owned capture and release devices. Enforce [Q.SEC-05](#qsec-05--logout-revocation-and-failure-lifecycle) even while media still flows and the server cannot acknowledge leave. Merely navigating to another account must not transfer the active call's ownership or start another capture session; preserve its visible account identity as specified in [AUTH-006](../product/accounts.md#auth-006). Application permission UX must sit above native platform requirements. [Apple capture authorization](https://developer.apple.com/documentation/avfoundation/requesting-authorization-to-capture-and-save-media)

Acceptance: record native capture state, outbound media and permission transitions on macOS. Test ringing, direct acceptance, voluntary conference join, denial/revocation, screen sharing, reconnect and teardown. A screenshot of a mute icon is insufficient.

## Q.SEC-10 — Recording, E2EE and changing permissions

Evaluate capability, server configuration, conversation policy, session and effective permission together. Unknown restrictive flags must not silently enable actions. Revalidate before joining, expanding media publication and resuming after reconnect; a version match or a previously enabled button is not authority. Restrictions take effect on observation and through the bounded freshness contract below, not through assumed instantaneous knowledge of unseen server changes.

### Required bootstrap policy decision

Before `G.BOOT` passes, a reviewed architecture decision must bind the following obligations to concrete modules/interfaces for each supported server/signaling profile. This specification selects **no numeric freshness defaults**. The decision must provide finite numbers, authoritative sources and tests; leaving these fields unresolved blocks bootstrap acceptance.

| Contract field | Required decision and observable rule |
| --- | --- |
| Owner and scope | One policy authority owns account/host/room/call-generation-scoped decisions; name its protocol refresh, timer and media-enforcement owners. Old-generation callbacks cannot extend current authority. |
| Authoritative sources | Map each enforced property to supported capabilities/configuration, current room/effective permissions/recording state and signaling/session responses. Pin routes/fields and identify host authority for federation. A server-internal setting name alone is not a client read API. An unavailable required property is unknown unless a version-pinned protocol rule explicitly defines its absence. |
| Source validity | Prove which response or server event refreshes each property's value and how caching/propagation affects it. Full refresh must cover properties omitted from the capability hash. A hash match, unrelated heartbeat, RTP receipt or unvalidated cached response cannot renew their freshness. Document source propagation/cache limits; if no bounded usable source exists, suspend/leave the affected path or mark it unsupported rather than claim bounded enforcement. |
| Time bounds | Declare maximum source age `F`, maximum refresh-start interval `R`, request deadline `D`, conservative source propagation/cache allowance `S`, enforcement deadline `E` and capture-release bound for each restriction class. For periodic reads, require `S + R + D <= F`; include retry/backoff and scheduler tolerance inside these bounds. No timer reset on a failed/partial request. Use a monotonic clock and define sleep/wake expiry. |
| Freshness state | Each decision carries source provenance, generation and expiry. For a periodic read, use a conservative source-time basis no later than request start minus `S`, with expiry at that basis plus `F`; define an equivalently justified basis for events. Receipt time alone cannot make delayed/old data fresh. The effective decision expires at the earliest required source expiry. Distinguish fresh-allowed, fresh-denied, unknown and stale; unknown/stale cannot grant new publication, join or permission expansion. |
| Invalidation and suspension | Known restrictive change invalidates affected grants as soon as observed, with enforcement within `E`. Lost control connectivity marks policy degraded and triggers refresh; if an existing grant is still usable, it expires no later than its original `F` bound. Without complete renewal, stop affected capture/publication by expiry plus `E`, even if RTP continues. Leave locally when membership, consent or required encryption cannot be established; a grid limit instead restricts subscription/rendering as its contract specifies. No successful server leave is required for local suspension. |
| Recovery and budget | Fresh state alone cannot unmute, enable a camera or restart a stopped share. Require valid continuity/user intent or deliberate rejoin and preserve prior mute/off settings. Specify refresh coalescing, request/byte/wakeup ceilings and failure backoff; reconcile their cost with [Q.PERF-04](performance.md#qperf-04--idle-energy-and-network-behavior) before approval. |

For a change not signaled to the client, the decision must derive the maximum exposure bound from its source propagation/cache allowance, refresh cadence/deadline and enforcement deadline. If refresh fails, the old grant expires independently. This is a limit on stale authorization under the declared source assumptions, not a promise to detect arbitrary unobservable server state. When a source invalidates on disconnect, suspension applies immediately on that observation plus `E`, without waiting out a cached grant. On wake, evaluate elapsed expiry before media may resume.

The pinned settings mark both `call_end_to_end_encryption` and `grid_videos_limit_enforced` as not changing the Talk version hash. Therefore hash-triggered refresh alone is insufficient during an uninterrupted call. The current-room API exposes per-participant permissions and recording-consent state; the bootstrap must verify the complete source mapping for both families. [Talk25 settings](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/settings.md), [Talk24 conversation API](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/conversation.md), [Talk25 conversation API](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/conversation.md)

Recording consent must be explicit where required, tied to the relevant call/policy and not preselected. Show recording transitions and stop/re-evaluate affected actions when policy changes. Do not infer absence of recording from missing optional data. The pinned Talk call API documents a consent parameter and refusal when required consent is missing. [Talk25 call API](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/call.md)

`call-end-to-end-encryption` advertises signaling support, not proof that this native client encrypts media end to end. The reviewed Talk settings associate call E2EE with the high-performance backend. A required policy that the client cannot satisfy blocks joining/publishing with an explanation; never silently downgrade. Claims require native interoperability evidence for negotiation, key distribution/rotation, participant changes and failure handling. Ordinary transport encryption, chat/file encryption and call E2EE must have distinct labels. Verify recording/E2EE combinations against the exact server and engine; do not invent compatibility. [Talk24 capabilities](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/capabilities.md), [Talk25 settings](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/settings.md)

Acceptance: test role loss, forced mute, lobby restrictions, changed consent, missing capability and required-but-unsupported E2EE. [V.POLICY.FRESH](verification.md#vpolicyfresh--ongoing-policy-freshness) adds non-hash changes during uninterrupted media, lost control connectivity, delayed responses and sleep/wake. Inspect source/expiry/enforcement timestamps and actual media behavior as well as UI state; no freshness pass is possible without the approved numeric decision.

## Q.SEC-11 — Dependencies and release integrity

Before distributing binaries, inventory Rust and native dependencies, versions, licenses, provenance and update owners; include media libraries and bundled assets. Review unsafe boundaries and relevant advisories, record scan limitations and resolve reachable high-impact findings. Verify signing, entitlements, notarization and update artifact authenticity on the delivered macOS build. Do not claim sandboxing, automatic updates or other release protections until they exist and are tested.

Acceptance: an independent coarse security review covers these artifacts and the above negative cases on the exact candidate commit. A successful dependency scan is not a penetration test or a complete security review.

## Q.SEC-12 — Disclosure and unresolved guarantees

Report vulnerabilities through the repository's **Security → Report a vulnerability** action when available: [repository security page](https://github.com/adapt2move/nextcloud-modern-talk/security). If unavailable, open a content-free issue asking maintainers for a private security contact; do not include exploit details, credentials or production data. Provide affected version/commit, platform, impact and a synthetic reproduction privately. [GitHub private reporting](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/report-privately)

This authoring phase has not verified that private reporting is enabled or established a response SLA. Before a product release, maintainers must verify a monitored private route and publish supported versions and response expectations. There is no security certification, completed external audit or guaranteed vulnerability-free state.
