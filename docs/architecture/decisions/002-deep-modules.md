# ADR-002 — Three packages, decision-owning modules

Status: proposed for `G.BOOT`, 2026-10-18. This is a small compilation graph with module contracts, not completed feature services. Q.SEC-01/02, Q.PERF-05 and Q.VER-02 govern its review.

## Graph and public surface

`talk-core` has no package dependencies. `talk-adapters` depends only on `talk-core`. `talk-bootstrap` depends on both and is the composition root. The eventual native application replaces this diagnostic consumer; it must preserve the renderer boundary. No generic event bus, plugin framework or feature-per-crate split is introduced. Sharing a crate does not permit bypassing the documented private ownership.

| Module / future owner | Small public interface | Complexity and data kept private |
| --- | --- | --- |
| Core domain / application owner | Typed local identities, bounded display values, `DesktopPort::{try_submit,take_snapshot}`, worker service ports | Account actor registry, intent validation, use-case ordering, cache indexes, delivery reconciliation, policy source aggregation and snapshot projection |
| Core policy / application owner | `Gate`, conservative `Lease`, typed decision/reason; future complete resolver consumes normalized observations | Capability/config/effective-permission precedence, dependency of each action on sources, consent/call binding and earliest expiry; no renderer recomputation |
| Identity / adapter owner | `IdentityPort::transition` with stage, activate, resolve, reconcile, exact attempted-item delete and suppression commands; redacted reports | Browser Login Flow, canonical origin/user binding, pending secrets, qualified vault operations, durable journal, provider status mapping and replacement serialization |
| Nextcloud / adapter owner | `NextcloudPort::request` with typed account probe, policy refresh, conversation reconciliation and leave requests | Endpoint construction, room-token/actor mapping, HTTP/OCS/DAV decoding, cookies/session tickets, TLS/origin validation, limits/retries, internal/HPB signaling and federation routing |
| Native media/platform / adapter owner | `MediaPort::{close_publication,stop}`; future separately reviewed call-control and frame-surface interfaces | Devices, permission prompts, WebRTC/codec stack, keys, native handles/FFI, worker IPC, encode/decode pools, publication gate, release watchdog and OS lifecycle |
| Presentation / native UI owner | Bounded `ViewSnapshot`, typed `UserIntent`, `SubmitError`; currently a diagnostic consumer | GPUI entity/window/focus/accessibility/layout objects, frame presentation handles and UI localization; no secrets, route construction or backend ownership |
| Composition / runtime owner | Construct explicit adapters and supervisors, start/stop root lifetime | Executor choice, native adapter wiring, account worker lifetimes, platform clock and process accounting registration |

`ProtocolReply` currently has no implemented conversation payload; `ReconcileConversation` is a typed reserved request and returns `Unconfigured`. `IdentityPort` references pending login IDs without exposing credential material. Network authentication will use an adapter-private borrowed credential within the identity owner and a prevalidated transport operation; no reusable secret/auth object is returned to core. `PolicyInputs` is a fixed-size source bundle: account validity, host configuration, local grid, room/effective permissions/recording, connected session and signaling profile each retain separate observation/lease provenance. It is never complete call authority by itself: the resolver must check sources, current session continuity, user consent and actual native encryption. There is no complete resolver yet. New operations must extend a cohesive owner and trace to a requirement, not become catch-all string requests.

## Invariants, errors and ownership

| Module | Invariants and failure/lifecycle contract | Test seam |
| --- | --- | --- |
| Core/application | Conversation unifies chat/files/call; presets and user groups are not new room types. Account and host scope every room/cache; `CallScope` adds runtime and call generation. IDs are local nonsecret handles; the future allocator must use OS randomness and reject collisions. A handle alone cannot authorize I/O. One serialized actor per account checks admission and rejects old completions. Shutdown never waits for the renderer. | Pure gate/admission checks now; later deterministic actor scheduler and injected ports. Product CT cases remain future. |
| Policy | Unknown is not false-by-default permission. Known restriction wins; only a fresh complete conjunction can allow. Server role/version names alone grant nothing. Restrictive observations invalidate before snapshot publication; old callbacks cannot renew leases. `Denied`, `Unknown`, `Stale` stay distinct. | Inject source provenance and monotonic epochs; lease tests now; source/event/precedence matrix later. [ADR-004](004-policy-media-lifecycle.md). |
| Identity | Runtime/attempt/vault generations are distinct. Only committed eligible metadata plus accessible confirmed item can restore. No overlapping account replacement; abandoned cleanup cannot delete retained older item. Cancellation is not settlement. Errors include operation, provider/category and redacted native code inside this owner; core/UI gets allowlisted category/recovery state only. | Scripted provider and journal fault points, then actual signed macOS Keychain qualification. [ADR-003](003-credential-lifecycle.md). |
| Nextcloud | Validate account/host/runtime before dispatch and callback delivery. Never forward a local app password to a remote host. Size limits precede allocation/decompression. A successful HTTP response is not durable-message or media success. Retry ambiguity requires reconciliation; errors are scoped action/session/account/transport evidence. | Frozen wire fixtures and disposable both-family servers, slow/malformed/cross-host responses; `NextcloudPort` double. |
| Media/platform | No capture on ringing. Mandatory permissions/consent/security before media admission. Frames and RTP bypass core's serialized snapshots. Account loss/expiry closes publication locally; stop owns device release and cannot be cancelled by the very account shutdown it completes. Unconfirmed release is failure, never “devices off.” | Timestamped stop observer and independent device/packet probe; controllable worker hangs, native permissions and termination. [ADR-004](004-policy-media-lifecycle.md). |
| Presentation | `PrepareConference` opens voluntary prejoin; `AcceptDirect` bypasses that interstitial, with all mandatory gates retained. No UI method blocks, polls a future, parses network data or does disk/vault/media work. UI drop detaches a window, not an account's running call; explicit root quit closes all accounts. | Snapshot/intent consumer and future native keyboard/accessibility tests. No HTML timing substitution. |

`Task` is a standard-library boxed `Send` future used at worker seams; it selects no executor. Construction and polling must be nonblocking; blocking OS calls move to bounded dedicated workers. `OperationContext` carries scope, deadline and cancellation. Adapters check it before dispatch and after completion. `PortError` contains only category, account reference and retry guidance. No provider message, URL, body or secret is propagated to the renderer. A `Busy` intent was not accepted; future UI must keep the user's input. An accepted intent means queued locally, not server success.

Account shutdown order is admission closure → generation invalidation/cancellation → media publication closure → priority local teardown → asynchronous durable suppression/cache cleanup and bounded remote leave. Credential replacement cancellation may preserve the older account runtime according to ADR-003; do not reuse account-loss shutdown for that narrower event. Root shutdown joins/terminates all owned workers, observes release, flushes permitted durable state and records unresolved cleanup. Other accounts remain independent. Process death is not a vault settlement oracle.

## Concrete resource contract

These are selected initial runtime limits; except bounded boundary values and the initial snapshot slot, they are not implemented. Increasing them needs IST/contract/performance review. Resource owner must expose nonsecret depth/age/eviction/overflow counters.

| Resource | Bound and overload/cancellation rule |
| --- | --- |
| Accounts and calls | At most 8 configured accounts, 1 publishing call per account, 2 active calls process-wide. Further joins return `Busy`; do not evict an existing call. Targets may expand after measured review. |
| UI intents | 64 per window; nonblocking try-send. Coalesce navigation/typing, reject overflow visibly. Leave/logout use a separate one-bit priority signal per account and never wait behind ordinary intents. |
| View snapshots | One latest slot per window, at most 100 conversation rows (256-byte labels) and 100 message excerpts (4 KiB each). Text counts UTF-8 bytes. Durable events live in the owner, not in replaceable snapshots. |
| Account worker | 128 ordinary control records/account, ≤8 KiB each; nonreplaceable overflow invalidates/reconciles the affected stream or closes admission. Restriction/stop signals use separate priority flags, never silent drop. |
| Network | 4 ordinary requests/account, 8 globally; additionally 5 reserved policy/probe requests per active call (coalesced sources). Control body ≤256 KiB decoded, ≤64 nesting levels, ≤4,096 scalar entries; reject oversize before unbounded expansion. One policy refresh/source outstanding. No service is enabled until required limits fit its protocol. |
| Message history/search | 50 messages/page, ≤4 resident pages/account plus visible projection; full individual message ≤128 KiB local hard cap subject to smaller server limit. One search/account, newer search cancels older. Larger supported content needs an explicit chunked/bounded path; no hidden truncation of the message itself. |
| Durable outgoing work | 100 pending messages/account and 16 MiB body total; explicit backpressure. Attachments are separately streamed from approved files. User drafts survive cancellation where policy permits; uncertain network outcomes reconcile before deliberate retry. |
| Transfers | 2 active/account, 4 globally; 256 KiB streaming chunk, queue of 16 descriptors/account. Cancellation closes streams and cleans owned temporary files; no body-sized buffering. |
| Caches | Decoded image cache 32 MiB process-wide; native GPU image cache 64 MiB; disk cache 256 MiB/account with policy-based expiry. Classified/sensitive exclusions override these maxima. These are sub-budgets, not a total-memory claim. |
| Media | 2 queued camera/screen frames per stream, at most 100 ms queued audio. Drop superseded video frames, report audio overflow and degrade/stop; never increase latency without bound. Opaque shared surfaces stay in media/renderer ownership, released on generation change. No CPU-frame copying through `ViewSnapshot`. |
| Vault/metadata | 1 provider operation/account, ≤2 provider workers globally, 8 queued descriptors/account; OS prompt outstanding blocks another prompt. Cancellation can stop admission immediately while uncertain provider work remains tracked. |

Policy scheduling/cost and frame subscription limits are in ADR-004; complete process accounting includes all these owners under the [measurement contract](../measurement.md). No bounded queue implementation or watchdog is claimed by the current types.

## Requirement-to-owner coverage

| Requirement family | Owner and acceptance references |
| --- | --- |
| AUTH-*, account isolation | Identity + account lifecycle; CT-001–003, V.AUTH.STORE/LOSS |
| COMP-*, CONV-*, MOD-* | Policy + Nextcloud + application; compatibility CT matrix, V.POLICY |
| MSG-*, CONTENT-* | Application state/cache + Nextcloud + bounded projection; messaging/content CT matrix |
| CALL-* | Call application coordinator + policy + media + Nextcloud; CT-011–015/018, V.CALL/POLICY.FRESH |
| DESK-*, JNY-*, STATE-* | Presentation + application + platform; native accessibility/journey tests |
| Q.SEC-*, Q.PERF-*, Q.VER-* | All owners within their boundaries; separate reviewers and PM acceptance |

This maps requirement families without claiming all feature operations have Rust signatures. Add only the reviewed slice's types after bootstrap acceptance and link exact feature/CT IDs. The accepted inventory and specifications remain the product source of truth.
