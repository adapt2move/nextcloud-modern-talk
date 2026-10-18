# Quality specification review — round 2

**Verdict: pass for the quality-text corrections only.** `QR1-01`–`QR1-05` are **verified fixed within the eight hashed quality documents below**. No residual objection to those corrections or blocking regression was found in this scope. This does **not** close their cross-document integration work or pass the whole `G.SPEC`: the six known product amendment groups and their independent verification remain prerequisites. No architecture, implementation, native security, integration or performance result is approved here.

Reviewer: independent delegated quality reviewer, round 2, 2026-10-18. I did not author or edit the candidate specifications, original review or remediation response. The brief was to reassess specification-level credential storage/fallback/session lifecycle, measurable performance, and disposable-server/independent acceptance gates, focusing on the five original findings and regressions introduced by their corrections. I read the [original review](quality-spec-review.md), [author response](quality-remediation-r1.md) and all eight quality documents in full. Product files are being reviewed separately; their known counterpart gaps are carried forward, not reported as new findings.

Candidate identity: draft file hashes, under `Q.VER-03`; no commit/base SHA, git state, build artifact, toolchain or dependency lock was inspected. Build/platform/fixture execution is `not run — outside this gate`. Final acceptance still requires confirmation against the final committed candidate. Locations below refer to the hashed bytes, not an unbounded future revision.

## Original finding dispositions

### QR1-01 — High: uncertain vault writes and older-session restoration

**Quality-text status: verified fixed.** Evidence: [Q.SEC-02/03/04](../quality/security.md#qsec-03--os-vault-contract), `security.md` lines 19, 33–55 and 74–78; [V.AUTH.STORE-01–06](../quality/verification.md#vauthstore--uncertain-storage-transitions), `verification.md` lines 74–83.

Runtime authorization, login-attempt and vault-item identities are distinct. A replacement is staged separately from the older retained item. Committed nonsecret intent precedes dispatch; restoration requires committed eligibility as well as a confirmed accessible item. Session-only activation requires durable suppression of all old/new restorable generations. Cancellation can deliberately retain only the named valid older session. Late completion cannot activate an abandoned attempt, and restart honors the committed disposition rather than enumerating vault items. Cleanup must prove both writer settlement and exact-generation absence, closing the early-absence/late-write race. Failed metadata updates prevent an unsupported restart promise; residual storage stays visible.

The six cases cover committed-but-unacknowledged writes, fallback/cancel and late success, older-session retention, crash boundaries, provider recovery and metadata failure/corruption. No quality-text objection remains. Product integration remains pending; concrete metadata durability, serialization and crash ordering belong to the required bootstrap interfaces. Native fault/Keychain tests remain required later.

### QR1-02 — High: confirmed account loss during active work

**Quality-text status: verified fixed.** Evidence: [Q.SEC-05](../quality/security.md#confirmed-account-authorization-loss), `security.md` lines 84–105; `Q.SEC-02/09`, lines 19 and 135–139; [V.AUTH.LOSS-01–03](../quality/verification.md#vauthloss--confirmed-account-authorization-loss), `verification.md` lines 87–93.

Confirmed account rejection now closes authenticated admission, invalidates the runtime generation, cancels account work and rejects obsolete callbacks. Publication is disabled in the media admission path; capture/device release has a required finite numeric bootstrap deadline. Neither local shutdown nor callback invalidation waits for remote leave, revocation or vault success. Stored-generation reuse is suppressed, with an explicit warning if durable suppression fails. Other accounts and permitted drafts survive; validated reauthorization still requires fresh policy/session checks and deliberate send retry or call rejoin.

The scenarios include active media plus a pending send, a second account, dropped leave responses, surviving transport, late successes/reconnects and suppression-write failure. They distinguish account loss from action denial, session expiry and network failure, and do not promise recall of transmitted requests. No quality-text objection remains. The bootstrap must define the rejection classifier, fan-out ownership and release deadline. The bound starts at confirmed observation, not at an unobservable server revocation time.

### QR1-03 — Medium: bounded policy freshness during uninterrupted media

**Quality-text status: verified fixed.** Evidence: [Q.SEC-10](../quality/security.md#required-bootstrap-policy-decision), `security.md` lines 143–167; `Q.VER-01`, `verification.md` line 18; [V.POLICY.FRESH-01–04](../quality/verification.md#vpolicyfresh--ongoing-policy-freshness), lines 97–104; [Q.PERF-04](../quality/performance.md#qperf-04--idle-energy-and-network-behavior), `performance.md` line 72.

The bootstrap decision must name policy/source/timer/media owners, map authoritative routes and fields for both server families and supported signaling profiles, and provide finite numeric `F/R/D/S/E` plus capture-release bounds. It must account for propagation/caching, retries and scheduler tolerance; periodic reads require `S + R + D <= F`. Conservative source-time expiry, earliest-source expiration, monotonic timing and wake-before-resume checks prevent delayed replies or unrelated traffic from renewing authority. Known restrictions invalidate grants on observation; failed refresh or half-open control expires them independently of working RTP. Required membership/consent/encryption uncertainty forces local leave; grid restrictions have an appropriately narrower subscription/rendering effect.

The cases require non-hash changes, control loss with live media, stale callbacks, sleep/wake, actual enforcement timestamps and refresh cost. Missing/unbounded sources cannot support a bounded-enforcement claim. `Q.VER-01` explicitly blocks `G.BOOT` for unfilled source/timing decisions. No quality-text objection remains: leaving numeric choices to that gate is intentional and sufficiently constrained. It is not evidence that sources, timings or safe enforcement have already been established.

### QR1-04 — Medium: missed process lifetimes and allocation peaks

**Quality-text status: verified fixed.** Evidence: [Q.PERF-01/02](../quality/performance.md#qperf-01--reference-environment-and-accounting), `performance.md` lines 11–25; [Q.PERF-07/08](../quality/performance.md#qperf-07--evidence-and-profiling), lines 92–104; [V.PERF.PEAK-01–02](../quality/verification.md#vperfpeak--complete-process-and-peak-evidence), `verification.md` lines 108–113.

One-second aggregate p95/sampled maxima are retained, with a separate qualified peak safeguard. Every owned lifetime, including short-lived, nested/reparented and abnormally exiting helpers, needs birth identity and high-water evidence. The sum `U` of lifetime maxima is explicitly an upper bound rather than a simultaneous measured peak. An excessive loose bound requires qualified tighter evidence; a low sampled maximum cannot rescue it. Observed ceiling breaches fail, and missing coverage is incomplete. Startup, warm-up and teardown are inside the peak ceiling.

Collector qualification must detect a subsecond touched allocation and between-tick helper, including a known ceiling breach, crashes and PID reuse, with overhead/error evidence. No quality-text objection remains. Actual API availability, lifetime collection and measurement accuracy on the selected Mac remain qualification obligations; retaining the provisional numbers under stricter accounting does not establish their feasibility.

### QR1-05 — Medium: reproducible activity and media profiles

**Quality-text status: verified fixed.** Evidence: [Q.PERF-02](../quality/performance.md#qperf-02--workload-budgets), `performance.md` lines 27–44; `Q.PERF-07/08`, lines 96 and 104; [Q.LAB-05](../quality/test-servers.md#qlab-05--reproducibility-evidence), `test-servers.md` lines 69–71; [workload manifest template](../quality/workload-manifest-template.md), lines 3–51; [V.PERF.WORKLOAD-01–02](../quality/verification.md#vperfworkload--frozen-and-validated-workload), `verification.md` lines 117–122.

Independent review must freeze the manifest and input hashes before acceptance runs. Required fields fix cadence/counts, search mix, paging/cache/residency, warm-up/reset, source assets, stream directions and codec/layer/quality tolerances. Local screen sender and receiver are distinct profiles retaining the camera load. Negotiated and delivered quality must validate throughout the run; materially lighter activity or media cannot inherit a baseline pass. Two runners must reconstruct matching normalized plans, with actual results recorded separately from frozen inputs.

The negative case deliberately changes workload, cache, quality and screen roles; an invalid run cannot become a pass by widening tolerances afterward. No quality-text objection remains. The template is correctly labelled unfrozen; schema/replay/validator ownership is a bootstrap obligation, while approved executable inputs and actual reconstruction are prerequisites to later performance acceptance.

## Regression and evidence checks

- **Coarse security, specification scope: pass.** The reviewed changes preserve intentional older-item retention, exact-attempt cleanup, narrow error classification and account isolation. Ringing still causes no capture; direct acceptance still bypasses device-prejoin, while voluntary conference joins retain it and mandatory permissions/consent/security checks remain applicable (`Q.SEC-09`, `V.CALL`). No unconditional memory-erasure or instant remote-revocation guarantee was introduced.
- **Performance, specification scope: pass.** The revised rules distinguish sampled p95, sampled maximum, qualified peak bounds, invalid workload, incomplete evidence and measured failure. Policy refresh/retry costs remain counted. No native benchmark or measured budget pass is asserted.
- **Verification/lab contracts: consistent within the reviewed scope.** `Q.VER-01/02/03/04/06/08` retain ordered gates, independent reviews, exact candidate identity, capability-based behavior and separately scoped evidence for both server families. `Q.LAB-03/04/05` keeps real media/profile evidence distinct from API observations and injected faults. Both report templates carry the revised obligations and mark absent runtime evidence outside specification/bootstrap gates. This is not the separate architecture or full function/compatibility review.
- **Independent read-only document checks:** eight quality hashes match the author's handoff; 34 unique requirement headings (`SEC=12`, `PERF=8`, `VER=8`, `LAB=6`) match the established ID set; 17 unique acceptance cases (`STORE=6`, `LOSS=3`, `FRESH=4`, `PEAK=2`, `WORKLOAD=2`) are present. All 45 relative-link occurrences within the quality directory resolve to files/heading anchors. Three product-link occurrences were excluded from that check. A private-host-path pattern scan found no matches; this is not a complete secret scan.

Focused primary-source checks on 2026-10-18 confirmed the two factual premises relevant to the corrections. The pinned settings table marks `call_end_to_end_encryption` and `grid_videos_limit_enforced` as not changing the Talk version hash. This supports the hash-only-refresh objection; it does not prove a complete client freshness API. [Official pinned Talk25 settings](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/settings.md)

Apple's header separately declares `phys_footprint` and `ledger_phys_footprint_peak`. The checked `main` reference is moving API research, not a pinned SDK or proof of access/coverage on macOS 15.7. The candidate correctly requires later collector qualification. [Official Apple task memory header](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/task_info.h)

No other external-source results from round 1 or the author response are represented as independently repeated in this round. Registry/package pins were not downloaded, installed or revalidated here. Document checks used standard-library hashing/parsing and manual contract review, not application tests.

## Remaining prerequisites and readiness scope

1. **Before whole `G.SPEC` acceptance — product integration author and project manager:** complete the [six existing product amendment groups](quality-remediation-r1.md#required-product-integration), obtain the separate product review/recheck and reconcile the resulting candidate. These are known integration prerequisites, not new QR2 findings. This review's closure applies only to quality text; it cannot be used to mark the unresolved account/call counterparts closed. Obtain final commit-bound confirmation under `Q.VER-03`.
2. **Before `G.BOOT` acceptance — architecture author and independent reviewers:** settle credential durability/staging/crash interfaces, account-loss classification/cancellation/media ownership and numeric release deadline; approve source mapping and numeric freshness/exposure/suspension/recovery/cost decisions; assign workload/validator and complete lifetime/peak collector contracts and qualification plans. None has passed through this document review. Full implementation must wait for the manager's bootstrap acceptance.
3. **Before native/integration/performance claims — implementation and verification owners:** execute the specified cases with actual Keychain, native capture/media, pinned disposable server profiles, qualified Mac collectors and independently frozen/reconstructed workloads. HPB/TURN pins, required hardware and actual source/adapter behavior remain future evidence. These missing runs do not block approval of the documentation correction; they block the corresponding product claims.

No new quality-text blocker or exception is proposed. No complete `G.SPEC`, `G.BOOT`, `G.PRODUCT` or `G.RELEASE` pass is recorded. Only this report was written. No specification/product/design/code or earlier report was edited; no agents, VM, service, git operation or publication were started. Read-only dev-box stats reported a new session with zero runtime/cost; no VM teardown was needed.

## Reviewed input SHA-256s

Repository-relative paths; hashes cover exact raw bytes. All ten inputs were checked again before handoff. This inventory identifies the draft review, not a committed acceptance candidate.

| Input | SHA-256 |
| --- | --- |
| `docs/reviews/quality-spec-review.md` | `26c8b5e69c13abcaf595afef9e16a341301b286c943a5606279e239f3cee88f7` |
| `docs/reviews/quality-remediation-r1.md` | `d4c55586c106ab4bc1048cc9fdbb2161333d21a9adea5e6df8378dac131fa946` |
| `docs/quality/acceptance-report-template.md` | `18289d1282953539641532693534895671b6a190638f89c9f7a0994e89375e54` |
| `docs/quality/index.md` | `cda4671cef04e7a912212acdf618427b7986d2499d26ead3ae61fd3ef0b51876` |
| `docs/quality/performance.md` | `89ef72053f3a2333b642bb9f1d4c0cbaf79d4f26e02b32892740797ca0e1523f` |
| `docs/quality/review-report-template.md` | `d78ca6aecd78a5d78875d81d49e491f2d495d1a0865bc50bf2fd5dffb25ea792` |
| `docs/quality/security.md` | `4b7506eb4a3ac318752d68548db5d817bd9811ace86de7fbc87250c214ec1691` |
| `docs/quality/test-servers.md` | `a11bb36216c6c442a8d092d8bbb6ce4c582f79f58a3f6dae4bbfcd8301ad9dc0` |
| `docs/quality/verification.md` | `e4c60ce8e61896003a340e69e073eb6b22acdab3dbd2022a94547d29d5b8561a` |
| `docs/quality/workload-manifest-template.md` | `c1d0e53f496fe51dba5a2870565aeb550fdc6a1fad90de3b54434382452daba4` |

**This conclusion concerns specification corrections only. Product security, native integration and performance remain unverified.**
