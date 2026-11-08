# Verification and acceptance

This document defines verification requirements for claimed product behavior; it records no passed gate. The task owner selects proportionate checks and review scope. Routine documentation and bounded settings work do not automatically require repeated independent reviews. Product/release acceptance needs evidence for its actual claims.

[AMEND-AUTH-STORAGE-01](../product/accounts.md#spec-amendment--amend-auth-storage-01) defines the native/private-file storage requirements. The AUTH cases below specify required coverage; results belong to exact candidate and platform evidence. A requirement alone establishes no provider, platform or feature pass.

## Q.VER-01 — Product and release acceptance

Choose checks for the changed behavior, failure paths and resource lifetime. Public
claims require actual behavior on the named server and native platform; compilation,
a screenshot, a mock or an HTTP health check alone cannot establish integration.
Release acceptance also requires package/signing/update-policy checks, current
limitations, a disclosure route and no blocking findings.

Credential crash ordering, account-loss classification and local media shutdown
must preserve [security requirements](security.md). An enforcement claim requires
finite, reviewed [policy freshness](security.md#required-bootstrap-policy-decision)
bounds and authoritative sources, including non-hash changes. Performance claims
require the [frozen workload and complete collector](../architecture/measurement.md).

## Q.VER-02 — Independent reviews and decision authority

The owner chooses review coverage for the changed behavior and risks: architecture
and lifetime, functionality and compatibility, security, and measured performance
where claimed. Reviewers inspect the exact candidate and checks, independently of
what they approve. A small reversible edit does not require four reports or repeated
rounds. Green CI is not independent product acceptance or an external security audit.

Unauthorized capture, account leakage, broken core workflows, missing required
checks and unmet advertised budgets block acceptance. Lower-impact exceptions need
an owner, expiry/milestone and narrowed claim; they cannot turn an unrun test into a pass.

## Q.VER-03 — Exact candidate and evidence identity

Record the commit, artifact hash, toolchain/lockfile, platform, fixture/server profile,
commands, outcomes and limits in the issue, PR or CI artifacts. Diagnostic dirty runs
also name their patch hash. Changes invalidate affected results until revalidated.
Findings name severity, reproduction, expected/actual result and affected requirement;
reviewers inspect fixes against the final candidate. Keep secrets and private paths out
of public artifacts. Missing, failed and inapplicable checks stay distinguishable.

## Q.VER-04 — Support claims and traceability

[Known limitations](../limitations.md) owns delivered scope; the product requirements
own intended behavior. Link changed requirements to checks and actual results in the
PR or release record rather than maintaining a parallel feature-status ledger.
Distinguish unsupported client behavior, server-disabled behavior and unverified
integration. Optional recording, federation, SIP, providers and E2EE need separate checks.

For claimed integration, exercise the core scenario on both supported server families
and name exact patches, capabilities, policy and backend versions. One patch pair does
not verify every 34.x/35.x deployment; backports need their own compatible contract.

## Q.VER-05 — Mandatory cross-cutting scenarios

These are verification groups, not another product specification. Attach the owning product IDs and actual expected behavior before execution. Apply them to the increment's claimed features and any restricted states it can encounter; record out-of-scope groups explicitly. A chat-only increment need not prove unimplemented media, but must not expose an unsafe join action or claim call support.

| Group | Required cases |
| --- | --- |
| `V.AUTH` | External-browser login/2FA, cancellation/expiry, app-password rejection, server loss versus wrong credentials, two accounts, locked/missing/denied/corrupt/unavailable native store, unencrypted private-file restart and owner/mode/DACL/path validation, explicit session-only choice, backend-qualified migration/recovery, logout/revocation/deletion failure; uncertain-write and account-loss cases below |
| `V.ROOM` | 1:1, group, public guest/password access and note-to-self; guest access versus discoverability; NextCloud groups as invitation sources; presets/forced values; denied admin/moderator actions; unknown type/attribute preservation with safe restrictions |
| `V.CHAT` | Claimed message/file workflows, pagination, ordering/deduplication, unread state, interruption and expiry; optimistic rollback, queue capacity, staging/dispatch crash boundaries, fresh reconnect validation and read-only uncertainty recovery under MSG-003 |
| `V.CALL` | Ringing with zero capture; direct-call accept without device-prejoin; voluntary conference join with prejoin; OS permission prompts/denial/revocation, lobby, recording consent, camera/microphone/screen intent, active capture indicator and teardown |
| `V.MEDIA` | Actual bidirectional audio/video, device switch, screen share, internal signaling and configured HPB/TURN, forced relay, reconnection, participant changes, required/unsupported E2EE and recording combinations |
| `V.POLICY` | Capability removed/restored; effective role lost mid-session; forced mute; changed limits/recording policy; unavailable external service; new unknown flags; account/conversation-host cache separation; bounded freshness during uninterrupted media and control loss below |
| `V.FAILURE` | Timeout, malformed/oversized payload, 401/403/404/409/429/5xx as applicable, retry guidance, DNS/TLS failure, offline/sleep/wake, server restart, cancelled transfer and late callbacks after logout |
| `V.PRIVACY` | Canary-secret scans of logs/URLs/cache/support bundles, links/attachment defenses, notification content, cache deletion and separate guest/account state |
| `V.NATIVE` | macOS keyboard/focus/accessibility and permission UX for claimed flows; native window/media evidence; no embedded WebView; future OS adapters qualified separately |
| `V.PERF` | Frozen workload reconstruction/actual-profile validation and complete lifetime/peak collector qualification below, then native budgets and regression evidence |

A controlled “downgrade” means switching between isolated supported profiles or removing capabilities/configuration/permissions. Never downgrade a newer server's database in place. Run real server configuration changes where available; use labelled contract fixtures/fault injection for malformed or unavailable states. Fixture results supplement real integration evidence and must not be reported as server observations. Refresh on capability hash changes, relevant reconnect/join/policy events **and** the approved uninterrupted-call schedule. Not every setting changes that hash. [Talk25 settings and hash column](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/settings.md)

For a durable chat/cache claim, exercise process restart before and after staging commit, dispatch-marker recovery, failed storage, full capacity without intent eviction, restricted/session-only exclusions, account isolation and logout purge. An unresolved send/forward outside the 200-message read window or uncorrelated scheduled creation must remain retained without another POST. These are acceptance conditions, not evidence that the candidate passed.

The scenarios below define required failure and lifetime coverage. Record actual results separately, using native adapters/media/servers where applicable and labelled fault injection for otherwise inaccessible states.

### V.AUTH.STORE — Uncertain storage transitions

Applies to [Q.SEC-03/04](security.md#qsec-03--os-vault-contract) and `AUTH-003/004/005/008/009`, covered by CT-003. Use separate attempted `N` and retained `P`, each with its exact account/profile/backend/store binding; these are nonsecret identities. Apply -01–06 to native and file providers, including cross-provider replacement. Runtime evidence must label actual OS/provider/filesystem tests separately from injected faults and real R34/R35 authorization; static authoring runs none of them.

| Scenario | Stimulus and expected result |
| --- | --- |
| `V.AUTH.STORE-01` | Let the provider commit `N` but lose/delay completion. Choose session-only. Before new session activation, durable account suppression covers `P` and `N`; restart restores neither. Cleanup targets only `N`; `P` remains intact. Residual-storage uncertainty stays visible until settlement and cleanup are proved. |
| `V.AUTH.STORE-02` | Cancel new-account authorization while the write is pending, then deliver late success. Remain unconnected; no callback grants restoration eligibility or activates `N`. Clean up `N` by exact identity. An early absent-item read followed by a late write cannot count as completed cleanup. |
| `V.AUTH.STORE-03` | Cancel reauthorization and deliberately retain valid persistent `P`. Only a committed cancel-and-retain transition permits `P` restoration; `N` stays ineligible even if cleanup is pending. Existing `P` is neither deleted nor overwritten; its valid active session may continue. |
| `V.AUTH.STORE-04` | Crash before/after intent persistence, provider commit, activation, fallback/cancel selection and cleanup acknowledgement. Restart honors the last committed disposition; unresolved intent blocks automatic restoration, including of `P`. No generation is selected by provider enumeration alone. Inject late completion and repeated cleanup after recovery. |
| `V.AUTH.STORE-05` | Deny provider reads/deletes during chosen session-only fallback with `P` present; recover the provider later. Preserve `P`, retain attempted-generation cleanup status and account suppression, and do not silently save/restore. Explicit native/file save must reconcile the abandoned write and obtain a new valid save intent. |
| `V.AUTH.STORE-06` | Fail durable intent/suppression metadata and separately corrupt/miss restoration metadata. No native/file provider write may precede committed intent; no replacement may activate under an unfulfilled session-only restart promise. Explain repair/cancel outcomes; do not create secret recovery exports or guess restoration eligibility. An ordinary private-file save requires its own valid committed intent. |
| `V.AUTH.STORE-07` | On Linux separately remove the session D-Bus and run a bus without Secret Service; verify absence is not classified as a locked collection or missing credential. Offer durable unencrypted private-file storage. Authorize a synthetic account, choose it, confirm file durability plus backend-qualified eligibility, exit/restart with the service still absent and restore that credential without another browser login solely because of store absence. Verify actual authenticated account binding on R34 and R35 when the adapter exists. No primary-password or master-password/KDF prompt. |
| `V.AUTH.STORE-08` | On POSIX inject wrong directory/file owner, `0755` directory, `0644` file, extra ACL access, unsafe ancestor, symlink/hard-link/traversal and swapped-path races; also shared/synced/temp locations or unsafe XDG overrides. Reject before secret read/write, never chmod through a link, broaden permissions or overwrite another object. Positive creation has owned `0700` directories/`0600` files from first secret write, including private staging siblings. Check safe repair/retry, retained P and honest session/cancel alternatives. |
| `V.AUTH.STORE-09` | On Windows inject DACL creation/readback failure, wrong owner SID, null/broad/inherited DACL, junction/reparse redirection, and a filesystem without enforceable ACLs. File mode must not claim privacy or persist there. A read-only flag alone cannot pass. Positive case verifies the owner-restricted DACL on directory, staged and final files before secret bytes are admitted. No portable POSIX-mode assumption, temporary/shared fallback or bypass on ACL failure. |
| `V.AUTH.STORE-10` | Lock a real Keychain/Secret Service collection; separately deny/dismiss its prompt and deny Windows credential access. Classify using qualified native evidence, with at most one outstanding prompt and no background re-prompt after refusal. A valid memory session continues if still admitted; an inaccessible credential is never copied into a file. Deliberate file/session choice uses available validated memory or fresh browser authorization. No failure automatically changes backend or reports wrong NextCloud credentials. |
| `V.AUTH.STORE-11` | Start with committed file eligibility, restore it, then make the native store available/unlock it and restart again. Keep the file binding/mode and create no native credential. Repeat with session-only: no backend restores. Populate an old native P and an abandoned N, including a colliding generation number in another provider; availability/enumeration never merges generations or reactivates either. A changed/missing store binding requires recovery rather than a guessed default. |
| `V.AUTH.STORE-12` | Deliberately migrate file→native and native→file, and explicitly save a session-only credential. Reconcile earlier writers and create fresh target N under committed destination intent; atomically commit sole target mode/eligibility only after confirmed storage. Fail/cancel/crash before and after intent, target install, activation and source retirement. P stays intact before authorized retirement; failed migration follows the last committed disposition, and only committed validated cancel-and-retain restores P. Failed source deletion after target activation reports residual cleanup without enabling both. A delayed receipt cannot supersede newer suppression or choice. |
| `V.AUTH.STORE-13` | Race two processes/account writers and a source/target save; lose the profile lock; inject short writes, disk/quota exhaustion, flush/atomic-install failures and an existing destination. No overwrite of P/collision or overlapping account write; no partial record becomes eligible, cross-volume copy fallback or false saved status. Crash around each file/ledger flush/install boundary. Reacquisition/early absence alone cannot settle a pending writer. Hit 16 KiB record/16-record-account and existing worker/descriptor limits: visible backpressure, no truncation, unbounded retries or reclamation of retained/uncertain items. |
| `V.AUTH.STORE-14` | Truncate/corrupt/oversize a file; mismatch schema/account/backend/generation; remove/corrupt ledger metadata; leave private staging/orphan files. Startup reports recovery and cannot guess credentials from enumeration or overwrite malformed records. Exact cleanup remains settlement-gated and preserves P. Feed canaries through native raw-byte error variants and file parsing failures: secret bytes, raw provider errors and file contents never reach logs, environment, clipboard, diagnostics or shared caches. |
| `V.AUTH.STORE-15` | With file credentials in A and independent native/file B, cancel an authorization/write, perform offline logout/removal and confirm server revocation separately. Deny file deletion and fail suppression metadata; deliver late native/file writes and stale callbacks; restart with residual items. Local A admission closes, backend-qualified cleanup stays pending, no abandoned N or rejected generation revives, B survives. Apply V.AUTH.LOSS's local media-stop checks when media exists; remote revocation, deletion and restart-suppression results remain distinct. |
| `V.AUTH.STORE-16` | Choose session-only despite available native/file persistence, including after a known non-write and an uncertain earlier write. Commit all-backend suppression before replacement activation; file permissions/OS recovery cannot silently save. Exit/restart restores neither P nor N. Current valid memory can continue while a save fails; failed suppression blocks replacement with an honest warning. No new secret file, content-cache key or durable content cache is created by session-only. Later explicit save requires reconciled prior work, fresh N, qualified binding and a new committed choice. |

### V.AUTH.LOSS — Confirmed account authorization loss

Applies to [Q.SEC-02/05/09](security.md#qsec-05--logout-revocation-and-failure-lifecycle), `AUTH-007` and `CALL-014`.

Run the applicable loss cases with native and private-file credentials, retained items in the other backend and migration pending. Loss/suppression covers all provider bindings; no unencrypted-file exception weakens generation invalidation or local shutdown. Backend-lab API checks alone do not prove native storage or capture behavior.

| Scenario | Stimulus and expected result |
| --- | --- |
| `V.AUTH.LOSS-01` | Account A is publishing microphone/camera/screen with a send/transfer pending; B has an independent session/draft and active permitted work. Revoke A on the disposable server, then produce the protocol observation that confirms account rejection. From that observation, A's generation is invalid, authenticated admission/retry stops, owned capture/publication stops within the approved release bound and queued media is discarded. Drop the leave response and preserve the media transport in the fault harness: local stop still completes. B and permitted drafts are preserved. |
| `V.AUTH.LOSS-02` | After shutdown, deliver old request successes, login/permission/media callbacks and reconnect triggers; restart with the rejected stored item. None restores access, marks an uncertain send as delivered from an obsolete callback or resumes capture. New validated authorization and fresh policies do not automatically replay sends or rejoin; a different identity cannot inherit the pending work. Report server-observed delivery separately because a transmitted request may already have succeeded. |
| `V.AUTH.LOSS-03` | Separately inject an action denial, conversation/signaling expiry, account rejection, timeout/5xx and a failed suppression-metadata write. The classifier selects the narrow documented scope; a room/action error cannot invalidate B or imply account revocation. Confirmed account loss still shuts A down locally if durable suppression fails, with an explicit unconfirmed-restart-suppression warning. |

### V.POLICY.FRESH — Ongoing policy freshness

Applies to [Q.SEC-10](security.md#qsec-10--recording-e2ee-and-changing-permissions), `CALL-006/009/011/014` and [Q.PERF-04](performance.md#qperf-04--idle-energy-and-network-behavior). Bind scenarios to the approved per-source/profile numeric decision before execution; report change, source observation, request start/completion, expiry, grant invalidation and actual media-stop/subscription timestamps.

| Scenario | Stimulus and expected result |
| --- | --- |
| `V.POLICY.FRESH-01` | During uninterrupted media, with no reconnect/join/hash change, change `call_end_to_end_encryption` and separately `grid_videos_limit_enforced` through supported server administration. The complete refresh observes/enforces the new restriction within the declared source/refresh/enforcement bound, or the affected grant expires and suspends safely. Prove the hash did not change; enforce actual encryption/publication/subscription behavior, not just labels. |
| `V.POLICY.FRESH-02` | Block policy/control traffic while RTP continues; include a half-open connection that supplies no explicit disconnect. Failed, partial, cached-only or hash-only responses do not extend expiry. Affected capture/publication suspends by existing grant expiry plus `E`; membership/consent/required-encryption uncertainty leads to local leave. Lost server leave responses cannot delay this. |
| `V.POLICY.FRESH-03` | Deliver delayed/out-of-order/old-generation responses and unrelated heartbeats; suspend/resume the OS beyond expiry. No stale response renews authority or clears a known newer restriction. Expiry is evaluated before resumed media. Restored control requires fresh source checks and valid user intent; no automatic unmute/camera/share restart. |
| `V.POLICY.FRESH-04` | Exercise restrictive pushed events, source propagation/cache limits, retry storms and both server families/signaling profiles. Measure enforcement relative to observation/expiry and total refresh bytes/wakeups. An unbounded/unavailable source blocks the affected support claim; a budget excess requires review, not dropped refreshes or excluded traffic. |

### V.PERF.PEAK — Complete process and peak evidence

Applies to [Q.PERF-01/02/07](performance.md#qperf-01--reference-environment-and-accounting).

| Scenario | Stimulus and expected result |
| --- | --- |
| `V.PERF.PEAK-01` | Inject a touched allocation burst and a short-lived owned helper entirely between one-second ticks, including a case whose peak breaches the selected ceiling. Lifetime inventory and qualified high-water/peak evidence detect the load; a sampled maximum below the ceiling cannot pass the breach. Record qualification fixture/measurement uncertainty and collector overhead. |
| `V.PERF.PEAK-02` | Repeat with startup/teardown allocations, helper crash/forced exit, nested/reparented helpers and PID reuse. No owned lifetime disappears or is merged. Missing peak data yields incomplete evidence, not zero; a loose sum of lifetime maxima above the ceiling requires a qualified tighter bound and is never labeled a simultaneous measured peak. |

### V.PERF.WORKLOAD — Frozen and validated workload

Applies to [Q.PERF-02/07/08](performance.md#qperf-02--workload-budgets) and [Q.LAB-05](test-servers.md#qlab-05--reproducibility).

| Scenario | Stimulus and expected result |
| --- | --- |
| `V.PERF.WORKLOAD-01` | Two independent runners reconstruct fixtures, initial cache/page state and normalized activity/media plans from the approved frozen manifest. Input hashes and planned counts/directions/quality match. Each actual run reports planned/issued/completed actions, residency/query evidence and negotiated/delivered stream profiles against the same tolerances. |
| `V.PERF.WORKLOAD-02` | Deliberately skip actions/pages, warm a declared cold cache, change search mix, lower media layers, disable a camera while sharing or swap screen sender/receiver. The actual-profile validator rejects that run for the original budget claim. A separately approved profile needs its own evidence; editing tolerances after the run cannot manufacture a pass. |

## Q.VER-06 — Proportionate execution and authorized lab use

Unit/contract tests exercise meaningful invariants, protocol parsing and failure transitions. Integration tests exercise adapters and real servers. Native scenario tests exercise OS devices/vault/UI; performance runs use the declared reference hardware. Do not inflate coverage with tests that merely repeat implementation constants or claim a mock is a real server.

The user has authorized disposable NextCloud/Talk servers. [The lab plan](test-servers.md) defines isolation, versions, secret handling and cleanup. Run services only within their assigned execution scope. Use a local development environment, an isolated container or CI. Clean up disposable services and secret-bearing fixtures after the run. Linux-server evidence cannot substitute for macOS capture/Keychain/performance evidence.

## Q.VER-07 — Current-state documentation and handoff

Update current limitations and public claims when a change affects them; do not add a public task diary for every internal edit. The private handoff records changed behavior, actual backend/native integration, checks run, unrun checks and remaining limitations. Update module/interface docs when implementation changes a contract. Preserve previous evidence with its original commit; do not relabel it as current.

Before publication, inspect documentation, screenshots, fixtures, traces and diagnostic artifacts for credentials, real customer data and private host paths. Confirm that design images/prototypes are identified as design artifacts and provisional budgets as targets. A public EUPL-1.2 project still needs accurate third-party attribution; do not import external source snippets to fill specifications.

Product acceptance needs accurate current-state documentation. Passing tests do not justify misleading support or release claims.

## Q.VER-08 — Review closure

When a review is assigned, preserve its brief, candidate, findings and unresolved
objections in the issue or PR. Rechecks account for earlier findings and intervening
changes. The owner's acceptance states the exact candidate, scope, checks and remaining
limits. Routine documentation work does not trigger extra review rounds by this rule.
