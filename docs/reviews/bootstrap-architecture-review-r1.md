# Independent bootstrap architecture review — round 1

**Verdict: changes required.** Two open architecture findings, **BAR1-01 (high)** and **BAR1-02 (medium)**, prevent a pass for this discipline. They concern the identity lifecycle interface and the consistency of the renderer boundary with its checker. Missing native adapters are expected at this gate and are not findings by themselves.

**This conclusion concerns bootstrap contracts/scaffolding only. Product security, media integration and native performance remain unverified.** It does not approve `G.BOOT`, publication or product implementation. The root project manager alone accepts gates after all independent evidence.

## Identity, independence and method

- Gate/discipline: `G.BOOT`, architecture/modules only; round 1, **2026-10-18**.
- Reviewer: the independently delegated architecture/modules reviewer for this assignment, using Codex. I did not author the candidate implementation, ADRs, accepted specifications, integration evidence or historical reviews. I wrote only this report and its [static evidence](evidence/bootstrap-architecture-r1/static-evidence.json).
- Candidate authorship: separately delegated bootstrap contributors, followed by the integrator and the assigned BI-001 documentation correction. The [integration report](bootstrap-integration-r1.md) attributes its work separately; Git records Maximilian as commit author. A distinct personal author identity for each source file is not established here.
- Original brief: public EUPL-1.2 native Rust/GPUI Talk client, macOS primary, no WebView; unified Talk conversations, capabilities/configuration/effective permissions, direct acceptance without voluntary prejoin, OS-vault credentials with explicit safe fallback, and specification → bootstrap → implementation gates. This review applies the mandatory [bootstrap handoff](product-spec-review-r2.md#mandatory-bootstrap-handoff), not a reduced compiling-skeleton definition of the product.

| Identity | Exact value |
| --- | --- |
| Reviewed candidate | `0b6e94db7faa4bee3d2ae88a5d2da2a5567e05be` |
| Candidate tree | `36133a4336cab291ca174ec9890b9a61c32ce599` |
| Immediate comparison/base, C2 | `c1cc08ee03ef154c495a20ad0d6c220a1889a3df` |
| Bootstrap source, C1 | `d55f39613337f0172b5c209235ae2f43b154a31d` |
| Bootstrap base/specification report continuation | `d1522594fc16ea884c436e73fe08b0fdabc4a14f` |
| Accepted specification input | `adfb44a2aab2e29ba6574c9dacc09475c534bccf` |
| Candidate source TAR SHA-256 | `989d3d14c3d411763f821fd3c6eb57cc3743f69162e79017b96dabd538daa4bd` |
| `Cargo.lock` SHA-256 | `97bc0a2b02ae77e013d45a47064b61b40dd6c6c6f495be5d245391cee8083cf8` |
| `rust-toolchain.toml` SHA-256 | `f715f440167e20fd8ea9afa0498c3d1ab80174cfcb8cbc895f106a34d907a1e1` |

The tracked input was clean, with no untracked files at the initial check. Reads used immutable `git show <candidate>:<path>` objects. `git log`, exact diffs and hashes confirmed the stated ancestry. C1→C2 changes only the integration report/evidence and IST; C2→candidate changes only the explicit `revision-05` anchor and its honest IST account. Rust, manifests, lockfile, architecture documents, checkers and lab harness are identical to C1. Accepted product/compatibility/quality documents, historical review inputs and licenses are unchanged; the additive design anchor is the inspected authorized exception. Concurrent reviewers' new reports are outside the candidate source.

The declared toolchain is Rust **1.99.1**, edition 2024, minimal profile with rustfmt/Clippy. The inspected lockfile contains only three local packages at 0.0.0 and no third-party Rust dependencies. No compiler or executable artifact was independently run or inspected here. The integrator's Linux compiler/test/diagnostic results remain attributed preliminary evidence, not my execution results.

Evidence records exact commands, selected input SHA-256s and unrun checks. Reproduce the source artifact identity with `git archive --format=tar 0b6e94db7faa4bee3d2ae88a5d2da2a5567e05be | sha256sum`. Source locations below refer to that commit. No project code, checker, test, service, VM command, upstream-source retrieval, commit, push or remote creation occurred in this review.

## Evidence assessment

| Scope / requirements | Static evidence and result | Limits |
| --- | --- | --- |
| ADR-001/002; Q.SEC-01/02; Q.PERF-05 | **Pass for graph and ownership design.** All manifests/lockfile, core modules, adapters and composition read. `talk-bootstrap → talk-core,talk-adapters`; `talk-adapters → talk-core`; core has no dependencies. Unsafe is forbidden. No feature-per-crate forest or generic event bus. | No implemented network/vault/media/renderer complexity whose encapsulation can yet be measured. BAR1-01/02 qualify the interface verdict. |
| Typed scope, cancellation, errors; AUTH-*; COMP-* | **Pass for the inspected primitives.** Distinct local account/host/actor/runtime/attempt/vault/call/clock identities; `CallScope` checks account consistency. `Admission` closes locally and shares cancellation. Error/decision enums distinguish unconfigured, unsupported, denied, uncertainty and stale policy. | These are caller-preconditioned primitives, not complete admission, source-validation or identity services. Identity lifecycle composition fails BAR1-01. |
| ADR-003; Q.SEC-03–05; V.AUTH.STORE/LOSS | **Fail at the public lifecycle seam — BAR1-01.** The ADR's durable ordering, exact-generation cleanup and shutdown decisions are substantive, but `IdentityPort` cannot express all of them under its stated context rules. | Actual journal, Keychain, crash settlement and account-loss fan-out **not run**. |
| ADR-004; Q.SEC-09/10; V.POLICY.FRESH | **Pass for ownership and timing design.** Separate account/local/host/room/session sources, explicit profile qualification, earliest expiry and known-restriction precedence. `S + R + D = 2 + 10 + 3 = F = 15 s`; enforcement is 250 ms; release is 1 s. The lease uses request-start + 13 s, rejects overdue/regressed/other-epoch responses, and expires strictly at its deadline. | Field/endpoint behavior, propagation bounds, native clock, source ordering and complete resolver **not independently qualified**. No supported native profile is enabled. |
| ADR-002/004; Q.PERF-04/05 | **Pass for resource design.** Finite accounts/calls, intent/control queues, requests, pages, transfers, caches, provider workers and media queues; priority stop/restriction signals; refresh request/byte/wakeup envelopes and failure behavior are assigned. Frames stay outside serialized snapshots. | Only boundary text/page limits and initial snapshot slot exist in code. Runtime bounds and cost estimates **not measured**. |
| UI boundary/checker; ADR-002; Q.VER-01/02 | **Fail — BAR1-02.** Public snapshot fields require data types unavailable through the sole permitted import path; the guard also rejects reexporting them. | Checker and compiler reproduction **not run**; static source derivation and requested verifier case below. |
| Measurement contract; Q.PERF-01/02/07/08; V.PERF.PEAK/WORKLOAD | **Pass for architectural assignment.** Frozen manifest, prepare/replay/validate/summarize, application/media observations and platform collector lifecycle have separate owners and bounded records. Birth identity, abnormal exits, PID reuse, startup/teardown, high-water coverage and incomplete evidence are distinguished. | No frozen acceptance workload, delivered-profile validation, collector qualification or benchmark executed. No performance-discipline verdict supplied here. |
| Composition/IST; Q.VER-03/04/07 | **Pass for inspected absence claims.** Every adapter request returns `Unconfigured`; intents are rejected; the diagnostic opens no account/device. All 140 features remain Planned. | Canonical checks, negative fixtures, CI and BI-001 execution are the designated verifier's responsibility. |

The [backend smoke](../testing/backend-smoke-2026-10-18.md), [testing entry](../testing/README.md) and [fixture contract](../../tests/lab/README.md) were read for evidence boundaries. The exact successful JSON hash is `6b85d61c3a1957b67a30fb710a46f21d96cd2f9dd0d7f43b526df13a4221d84d`; the harness hash is `ce5c9eaac74c701795587bbceb67bb885e2b93c74dca02d8004bddda0ca86678`. Both match candidate blobs. That binds existing author-run NC34.0.5/Talk24.0.6 and NC35.0.2/Talk25.0.6 evidence; it does not independently validate those runs or native behavior. The source mappings in ADR-004 were assessed for architectural scope/ownership and conservative failure handling; cited upstream files were not fetched again.

## BAR1-01 — High: identity lifecycle is not representable through its declared seam

**State: open. Owner: a separately delegated identity/architecture contributor; independent verifier owns execution.** Affects AUTH-003–005/007/008, Q.SEC-02–05, Q.VER-01, V.AUTH.STORE-01–06 and V.AUTH.LOSS-03. Locations: [ports.rs](../../crates/talk-core/src/ports.rs), lines 60–64 and 90–114; [ADR-002](../architecture/decisions/002-deep-modules.md), lines 27 and 32–34; [ADR-003](../architecture/decisions/003-credential-lifecycle.md), lines 12–19, 35–50 and 69–71.

The ADR says `IdentityPort` exposes its semantic transitions. The actual enum has only `Stage`, `Activate`, `Resolve`, `Reconcile`, `DeleteAbandonedAttempt` and `SuppressRestoration`. Its required user decisions and lifecycle contexts are missing:

1. **Reauthorization with retained P and uncertain attempted N:** select deliberate cancel-and-retain. There is no command carrying this choice. `Activate(StoreIntent)` commits eligibility for N; `SuppressRestoration(AccountId)` suppresses all generations; `Reconcile` expressly cannot change the selected disposition. None specifies the required atomic abandoned-N marker plus `Eligible(P)` transition. Session-only also lacks a command distinguishing deliberate in-memory activation from suppression without activation. Inferring either choice from cancellation would contradict the accepted explicit-choice contract.
2. **Cold restoration or recovery:** no validated runtime exists yet. Nevertheless both the common `OperationContext` and `Resolve(AccountScope)` require an `AccountScope` containing `RuntimeGeneration`. ADR-003 defines that generation as a validated in-memory authorization lifetime and issues a fresh runtime only after activation. No pre-runtime identity/recovery context or explicit alternative sequence is defined. Fabricating a local generation value is possible in Rust but does not resolve that contract contradiction.
3. **Confirmed account loss:** admission closes and the shared runtime token is cancelled before identity suppression/cleanup. The same `IdentityPort` still requires the common context, which ADR-002 tells adapters to check before dispatch and after completion. There is no specified cleanup authority/lifetime that survives invalidation. Supplying a fresh token or bypassing scope checks might be an implementation choice, but its ownership and allowed operations have not been decided. The media stop seam explicitly avoids this coupling; the identity seam does not.

**Expected:** the mandatory durable transitions can be selected explicitly and performed under the correct authority, before authorization or after its loss where required, without resurrecting a runtime or treating cancellation as user consent. **Actual:** prose supplies those outcomes while the concrete public interface omits the decisions or requires a contradictory lifecycle context. This is a bootstrap contract defect, not an observed credential leak in the unconfigured diagnostic. Implementing around it would require undocumented side channels or a substantive interface redesign after gate acceptance.

**Required correction:** reconcile the typed interface and ADRs now. Define pending-login, restoration/recovery and authenticated-operation scopes as needed; assign a bounded supervisor-owned lifetime for durable suppression and abandoned-write settlement after account cancellation. Expose explicit session-only and cancel/retain decisions, or an equally explicit typed transition carrying those choices, with exact account/attempt/generation correlation and durable acknowledgement semantics. Keep journal/provider execution private. Specify that dropping/cancelling a caller cannot discard the provider ledger, authorize late activation or suppress required recovery work.

**Acceptance/requested verifier cases:** demonstrate at contract level (without implementing Keychain) a fresh-account attempt and cold restoration with no previously live runtime; uncertain N → session-only; uncertain N → deliberate retained P; account loss → durable suppression after cancellation; and late settlement that cannot activate N. Check that old runtime work stays rejected and unrelated accounts survive. Retain V.AUTH.STORE/LOSS as future native fault tests. Update the affected ADRs and [IST](../status.md) in the remediation candidate. Author response/remediation commit: **none supplied**. Executed reproduction/recheck: **not run**; this finding is based on the exact signatures and state-transition contracts above.

## BAR1-02 — Medium: the allowed renderer dependency cannot name its own data types

**State: open. Owner: a separately delegated architecture/governance contributor; independent verifier owns execution.** Affects ADR-002, Q.VER-01/02 and the typed bounded renderer boundary. Locations: [ui.rs](../../crates/talk-core/src/ui.rs), lines 4–7, 24–27 and 37–43; [check_architecture.py](../../tools/check_architecture.py), lines 101–110; ADR-002, lines 7 and 16.

`ConversationRow.can_join` is `policy::Decision` and `ViewSnapshot.persistence` is `identity::PersistenceState`. The UI module imports both privately. Presentation is allowed to import only `talk_core::ui`; the checker rejects direct imports from `policy`, `identity`, `ids` or `bounded`. It also rejects every `pub use` in `ui.rs`, including safe data-only reexports.

**Static reproducer for the designated verifier, not executed here:** a normal pure presentation consumer needs to distinguish allowed, denied, unknown and stale states:

```rust
use talk_core::ui::ConversationRow;
use talk_core::policy::Decision;

pub fn join_is_available(row: &ConversationRow) -> bool {
    row.can_join == Decision::Allowed
}
```

The direct `policy` import matches the forbidden expression at checker line 109. Replacing it with `talk_core::ui::Decision` refers to a private import. Adding `pub use crate::policy::Decision` to the UI boundary matches the separate prohibition at line 106. The same conflict applies to structured persistence-state rendering. These are predictions from source inspection, not captured compiler/checker output. Equivalent grouped imports may evade this lexical checker; such a bypass would not satisfy the declared dependency rule.

**Expected:** an ordinary consumer can name and match all documented data/intent types while remaining inside the permitted dependency boundary. **Actual:** the current eight-line diagnostic works by discarding its snapshot, leaving the incompatible rules unexercised. This blocks the intended usable interface, without requiring GPUI or any adapter implementation.

**Required correction:** provide a complete, small presentation-facing data surface and a matching guard. For example, explicitly allow reviewed data-only reexports through `ui`, while retaining restrictions on worker ports, application/runtime owners, secrets, unbounded containers and frames. An equally explicit allowlist of dependency-free domain types is acceptable. Do not rely on import spelling to evade the rule or broadly disable boundary checks.

**Acceptance/requested verifier case:** compile a positive presentation fixture that matches both policy and persistence enums and uses the required typed intent identities through the approved surface; the boundary checker must accept it. The existing forbidden core-adapter/target-specific/presentation-adapter/unbounded-field/raw-route fixtures must still fail. Update the boundary documentation and IST. Author response/remediation commit: **none supplied**. Executed reproduction/recheck: **not run**.

## Discipline assessment and handoff

The three-package design is proportionate. Consequential transport, credential, timer, cache and media choices have named owners rather than facade crates. Private fields in the admission, lease, bounded-value and composition primitives support those boundaries. The resource table assigns finite bounds and overload behavior; frame transport and helper-process ownership stay separate from chat snapshots. The measurement plan gives full-lifetime evidence and actual-workload validation their own owners instead of treating a one-second sampled maximum as a peak proof.

The credential ADR correctly requires durable intent before write, separate N/P generations, account-wide session-only suppression, only deliberate committed retention of P, settlement before exact-N cleanup, and no restoration by vault enumeration. Its unresolved Keychain recovery barrier is honestly fail-closed. The account-loss classifier names narrow same-origin probe evidence and separates action/session/transport failures; local media shutdown is independent of remote leave and persistence success. BAR1-01 concerns how those decisions cross the actual interface, not reopening their accepted specification history.

Policy design accounts for non-hash E2EE/grid changes, local versus host authority, half-open control loss, sleep epochs, stale responses, minimum source expiry and counted refresh costs. Unqualified S=2 s propagation, native enforcement and collector profiles block affected future paths. `Gate`, `Lease`, `can_restore` and `Admission` are explicitly partial primitives with preconditions; they must not be promoted into a proved service. No current adapter issues a successful grant, restored session or device-release report. This review does not require absent future adapters to be implemented before bootstrap acceptance.

Function/compatibility, performance and coarse-security conclusions are **out of this report's scope**. All canonical project checks, checker fixtures, backend reruns and native/security/performance scenarios are **not run by this reviewer**. The independent functional verifier retains all execution and VM lifecycle ownership; no host workaround was used.

Historical [product](product-spec-review-r2.md), [quality](quality-spec-review-r2.md) and [design](design-spec-review-r2.md) round-2 conclusions remain preserved at their original scope. PR1-01–03, QR1-01–05 and DR-001–004 are not reopened as specification/design findings. Their mandatory bootstrap obligations were reviewed here. BI-001's additive anchor is statically consistent with its stated correction; its executed closure remains pending the designated verifier and is not claimed by this report.

**Recommendation to root:** do not accept the architecture portion of `G.BOOT` for `0b6e94db7faa4bee3d2ae88a5d2da2a5567e05be`. Assign BAR1-01/02 to separate implementation ownership, obtain a new exact candidate with ADR/IST updates and the requested verifier evidence, and return it for independent recheck. No exception is proposed. No product feature status change is warranted; all 140 remain Planned. PM acceptance and publication remain separate decisions.
