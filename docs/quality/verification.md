# Verification and acceptance

Status: required process; no gate is recorded as passed by this document. The project manager coordinates scope and makes the acceptance decision. Implementation is delegated. An implementer's own checks support a handoff but cannot be its final independent approval.

## Q.VER-01 — Ordered gates

| Gate | Entry and required evidence | What passing permits |
| --- | --- | --- |
| `G.SPEC` — Product specification | Stable feature IDs, domain model, observable behavior, acceptance criteria, capability/configuration/permission gates, error states and explicit scope/deferred features; independent review of consistency | Architecture/module/interface bootstrap |
| `G.BOOT` — Bootstrap | Accepted product baseline; Deep Modules ownership, narrow public interfaces, invariants, state/error/cancellation contracts, dependency direction, resource budgets and test seams; independent reviews below | Delegated product implementation |
| `G.PRODUCT` — Product increment | Delegated implementation handoff; exact candidate commit/artifact; real behavior and appropriate tests, both supported server profiles for claimed integration, native platform evidence, all four independent reviews, updated IST docs | Acceptance of the explicitly listed increment only |
| `G.RELEASE` — Distributed product | Accepted increment plus complete advertised feature matrix, packaging/signing/update-policy checks, disclosure route, sanitized reproducibility evidence and no blocking findings | The specific release and its stated claims |

Do not skip `G.SPEC` by turning screenshots into implementation tasks. Do not skip `G.BOOT` by calling a monolithic UI wrapper an architecture. A stub, mock, HTTP health check or compiling native window must be labelled for what it proves. Publishing a documented bootstrap does not authorize a “working Talk client” or measured-performance claim.

For `G.BOOT`, executable scaffolding, if present, gets appropriate format/compile/interface checks with results recorded; documentation-only work gets document/contract review. Do not manufacture runtime pass results for absent behavior. Product implementation starts only after the bootstrap acceptance decision, not merely after files are written.

Required bootstrap decisions include credential staging/restoration/crash ordering ([Q.SEC-03](security.md#qsec-03--os-vault-contract)), account-rejection classification and generation/cancellation/media shutdown ownership with a finite capture-release deadline ([Q.SEC-05](security.md#qsec-05--logout-revocation-and-failure-lifecycle)), and the complete numeric [policy-freshness contract](security.md#required-bootstrap-policy-decision). The latter must name authoritative sources, propagation/cache assumptions, refresh/expiry/enforcement bounds, suspension/recovery behavior and reconciled control-traffic/wakeup costs for both server families. A hash-only refresh or an unfilled timing/source decision blocks `G.BOOT`. Also assign the frozen-workload schema/owner and lifetime/peak collector interfaces and qualification plan ([Q.PERF-01](performance.md#qperf-01--reference-environment-and-accounting), [Q.PERF-02](performance.md#qperf-02--workload-budgets)); native measurement and adapter qualification remain product gates.

## Q.VER-02 — Independent reviews and decision authority

Every bootstrap/product candidate requires four separately attributable review reports. Each reviewer must be independent of the implementation or document they approve, receive the original brief and exact candidate, and inspect evidence rather than repeat the author's summary. A reviewer may cover multiple disciplines when competent, but each discipline has an explicit conclusion. The project manager must not replace delegated implementation with self-implementation or treat self-review as acceptance.

| Discipline | Bootstrap review | Product review |
| --- | --- | --- |
| Architecture/modules | Deep Modules hide consequential design decisions; APIs remain small; ownership, errors, lifetime, cancellation, account scoping and native adapters are coherent | Implementation preserves those boundaries; no secret/network/media logic has leaked into views; inspect resource ownership and failure paths |
| Function/compatibility | Feature IDs and acceptance criteria cover product decisions, degraded states and both target server families | Exercise claimed workflows on real pinned servers/native client, including negative and capability-change cases |
| Performance | Budgets are measurable; UI work, queue/cache limits, cancellation and profiling points are designed | Inspect release-build traces, resource boundaries, raw runs and regressions against [performance](performance.md) |
| Coarse security | Threats and credential/TLS/media/privacy interfaces have explicit safe behavior | Inspect actual adapters and error paths, redaction artifacts, permissions, dependencies and negative tests against [security](security.md) |

Use the [review template](review-report-template.md). “Coarse security review” is a scoped engineering review, not a penetration test, external audit or certification. Bootstrap performance/security passes apply to the plan/contracts only.

The project manager uses the [acceptance template](acceptance-report-template.md) after all required reports. Critical/high-impact security findings, unauthorized capture, account leakage, broken core workflows, missing required evidence or unmet advertised budgets block product acceptance. Lower-impact exceptions need an explicit owner, expiry/milestone and narrowed public claim; an exception cannot turn `not run` into `pass`.

## Q.VER-03 — Exact candidate and evidence identity

Every review names a full commit SHA, base SHA where relevant, clean/dirty state, artifact SHA-256, toolchain/dependency lock identity, platform and fixture/server manifest. Final acceptance uses a clean committed candidate. Diagnostic runs with local edits also record a patch hash; rerun affected checks after committing the candidate. Before the repository has commits, authors may supply a file-hash inventory for draft review; this is not final commit-bound approval. The manager must obtain confirmation against the final commit before accepting it.

Each finding records severity, affected requirement/module/feature IDs, reproducible steps, expected/actual result, evidence and scope limits. Remediation names the changed commit and regression evidence. Reviewers recheck the final candidate or explicitly carry forward an earlier finding/result after inspecting the intervening diff; the acceptance commit must match the reviewed commit. Code or dependency changes after review invalidate affected conclusions until revalidated. Do not make reviewers approve mutable branch names or screenshots alone.

Store sanitized commands, fixture hashes, logs/traces and outcomes in durable repository/CI artifacts with stable links and checksums where needed. Mark results `pass`, `fail`, `not run` or `not applicable` with rationale. A tool's successful exit code is not proof that the intended assertion ran.

## Q.VER-04 — Feature parity traceability

Maintain one authoritative product feature matrix. For each existing product ID, record: source/spec revision; target scope; capability/configuration/permission predicates; owning module/interface; positive and denied/degraded cases; platform; exact NC/Talk/profile; implementation state; test evidence; independent review; remaining limitation. Link quality IDs as cross-cutting requirements instead of duplicating the product catalog.

Required distinctions: `specified`, `bootstrap`, `implemented, unverified`, `verified`, `deferred`, `unsupported by client`, and `unavailable in this server profile`. A disabled server capability is not proof of client support. A truthful unsupported-state UI can pass its own acceptance criterion without passing the absent feature. Optional integrations, external-call services, E2EE, transcription, recording, federation and SIP require separate scope/evidence; do not infer them from chat or basic calls.

For each claimed integrated feature, run the same core scenario on NC34.x/Talk24.x and NC35.x/Talk25.x. Name exact patch versions and metadata-derived capabilities. A backported capability must work where advertised; absence on a newer server must disable/explain the action correctly. Do not infer “all 34.x/35.x verified” from one patch pair. Pin protocol evidence, for example [Talk24 capabilities](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/capabilities.md) and [Talk25 capabilities](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/capabilities.md).

## Q.VER-05 — Mandatory cross-cutting scenarios

These are verification groups, not another product specification. Attach the owning product IDs and actual expected behavior before execution. Apply them to the increment's claimed features and any restricted states it can encounter; record out-of-scope groups explicitly. A chat-only increment need not prove unimplemented media, but must not expose an unsafe join action or claim call support.

| Group | Required cases |
| --- | --- |
| `V.AUTH` | External-browser login/2FA, cancellation/expiry, app-password rejection, server loss versus wrong credentials, two accounts, locked/missing/denied/corrupt/unavailable vault, session-only restart, logout/revocation/deletion failure; uncertain-write transitions and confirmed account-loss scenarios below |
| `V.ROOM` | 1:1, group, public guest/password access and note-to-self; guest access versus discoverability; Nextcloud groups as invitation sources; presets/forced values; denied admin/moderator actions; unknown type/attribute preservation with safe restrictions |
| `V.CHAT` | Claimed send/read/reply/edit/delete/reaction/file workflows, pagination, ordering/deduplication, unread state, interruption and expired/removed content; exact behavior comes from the product matrix |
| `V.CALL` | Ringing with zero capture; direct-call accept without device-prejoin; voluntary conference join with prejoin; OS permission prompts/denial/revocation, lobby, recording consent, camera/microphone/screen intent, active capture indicator and teardown |
| `V.MEDIA` | Actual bidirectional audio/video, device switch, screen share, internal signaling and configured HPB/TURN, forced relay, reconnection, participant changes, required/unsupported E2EE and recording combinations |
| `V.POLICY` | Capability removed/restored; effective role lost mid-session; forced mute; changed limits/recording policy; unavailable external service; new unknown flags; account/conversation-host cache separation; bounded freshness during uninterrupted media and control loss below |
| `V.FAILURE` | Timeout, malformed/oversized payload, 401/403/404/409/429/5xx as applicable, retry guidance, DNS/TLS failure, offline/sleep/wake, server restart, cancelled transfer and late callbacks after logout |
| `V.PRIVACY` | Canary-secret scans of logs/URLs/cache/support bundles, links/attachment defenses, notification content, cache deletion and separate guest/account state |
| `V.NATIVE` | macOS keyboard/focus/accessibility and permission UX for claimed flows; native window/media evidence; no embedded WebView; future OS adapters qualified separately |
| `V.PERF` | Frozen workload reconstruction/actual-profile validation and complete lifetime/peak collector qualification below, then native budgets and regression evidence |

A controlled “downgrade” means switching between isolated supported profiles or removing capabilities/configuration/permissions. Never downgrade a newer server's database in place. Run real server configuration changes where available; use labelled contract fixtures/fault injection for malformed or unavailable states. Fixture results supplement real integration evidence and must not be reported as server observations. Refresh on capability hash changes, relevant reconnect/join/policy events **and** the approved uninterrupted-call schedule. Not every setting changes that hash. [Talk25 settings and hash column](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/settings.md)

The scenarios below are **specified, not run**. They add observable cases to existing quality/product IDs; they are not new claims of implemented behavior. Bootstrap reviews inspect their contracts/test seams. Product evidence uses actual adapters/media/servers where applicable and labels injected faults separately.

### V.AUTH.STORE — Uncertain storage transitions

Applies to [Q.SEC-03/04](security.md#qsec-03--os-vault-contract) and `AUTH-003/004/005/008`. Use separate attempted generation `N` and previously retained generation `P`; neither is a secret value.

| Scenario | Stimulus and expected result |
| --- | --- |
| `V.AUTH.STORE-01` | Let the provider commit `N` but lose/delay completion. Choose session-only. Before new session activation, durable account suppression covers `P` and `N`; restart restores neither. Cleanup targets only `N`; `P` remains intact. Residual-storage uncertainty stays visible until settlement and cleanup are proved. |
| `V.AUTH.STORE-02` | Cancel new-account authorization while the write is pending, then deliver late success. Remain unconnected; no callback grants restoration eligibility or activates `N`. Clean up `N` by exact identity. An early absent-item read followed by a late write cannot count as completed cleanup. |
| `V.AUTH.STORE-03` | Cancel reauthorization and deliberately retain valid persistent `P`. Only a committed cancel-and-retain transition permits `P` restoration; `N` stays ineligible even if cleanup is pending. Existing `P` is neither deleted nor overwritten; its valid active session may continue. |
| `V.AUTH.STORE-04` | Crash before/after intent persistence, provider commit, activation, fallback/cancel selection and cleanup acknowledgement. Restart honors the last committed disposition; unresolved intent blocks automatic restoration, including of `P`. No generation is selected by vault enumeration alone. Inject late completion and repeated cleanup after recovery. |
| `V.AUTH.STORE-05` | Deny vault reads/deletes during fallback with `P` present; recover the provider later. Preserve `P`, retain attempted-generation cleanup status and account suppression, and do not silently save/restore. Explicit save must reconcile the abandoned write and obtain a new valid save intent. |
| `V.AUTH.STORE-06` | Fail durable intent/suppression metadata and separately corrupt/miss restoration metadata. No vault write may precede committed intent; no replacement may activate under an unfulfilled session-only restart promise. Explain repair/cancel outcomes; do not create secret recovery files or guess restoration eligibility. |

### V.AUTH.LOSS — Confirmed account authorization loss

Applies to [Q.SEC-02/05/09](security.md#qsec-05--logout-revocation-and-failure-lifecycle), `AUTH-007` and `CALL-014`.

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

Applies to [Q.PERF-02/07/08](performance.md#qperf-02--workload-budgets) and [Q.LAB-05](test-servers.md#qlab-05--reproducibility-evidence).

| Scenario | Stimulus and expected result |
| --- | --- |
| `V.PERF.WORKLOAD-01` | Two independent runners reconstruct fixtures, initial cache/page state and normalized activity/media plans from the approved frozen manifest. Input hashes and planned counts/directions/quality match. Each actual run reports planned/issued/completed actions, residency/query evidence and negotiated/delivered stream profiles against the same tolerances. |
| `V.PERF.WORKLOAD-02` | Deliberately skip actions/pages, warm a declared cold cache, change search mix, lower media layers, disable a camera while sharing or swap screen sender/receiver. The actual-profile validator rejects that run for the original budget claim. A separately approved profile needs its own evidence; editing tolerances after the run cannot manufacture a pass. |

## Q.VER-06 — Proportionate execution and authorized lab use

Unit/contract tests exercise meaningful invariants, protocol parsing and failure transitions. Integration tests exercise adapters and real servers. Native scenario tests exercise OS devices/vault/UI; performance runs use the declared reference hardware. Do not inflate coverage with tests that merely repeat implementation constants or claim a mock is a real server.

The user has authorized disposable Nextcloud/Talk servers. [The lab plan](test-servers.md) defines isolation, versions, secret handling and cleanup. During this authoring phase only availability research and planning are performed: no VM, container, service or test server is created. Later builds/services/tests requiring the development VM use the existing agent-dev-box session; inspect status/cost before use and hibernate after cleanup. Linux-server evidence cannot substitute for macOS capture/Keychain/performance evidence.

## Q.VER-07 — Current-state documentation and handoff

Every implementation handoff updates IST/current-state documentation, feature matrix and public claims in the same candidate. List implemented behavior, actual backend/native integration, tests run, tests not run, known limitations and remaining target work. Update module/interface docs when implementation changes a contract. Preserve previous evidence with its original commit; do not relabel it as current.

Before publication, inspect documentation, screenshots, fixtures, traces and diagnostic artifacts for credentials, real customer data and private host paths. Confirm that design images/prototypes are identified as design artifacts and provisional budgets as targets. A public EUPL-1.2 project still needs accurate third-party attribution; do not import external source snippets to fill specifications.

Acceptance requires an IST link in the manager's report. Missing or misleading IST documentation blocks acceptance even when tests pass.

## Q.VER-08 — Closure and review rounds

For each independent review round, preserve the original brief, candidate, prior findings, author responses and unresolved objections. New rounds must not discard difficult findings. Track each finding through `open`, `fixed pending verification`, `verified fixed` or `explicitly deferred` with an owner and evidence.

The manager's final decision states the gate, exact commit and accepted scope; links all four reviews; resolves every blocker; records permitted exceptions and remaining product gates. A bootstrap publication may finish with “bootstrap accepted; product unimplemented/unverified.” A product claim may finish only with evidence for that claim. This document itself supplies neither decision.
