# Quality specification review — round 1

**Verdict: changes required for the reviewed specification baseline.** Two high and three medium findings remain open. Resolve the credential/session findings before accepting this portion of `G.SPEC`; put the policy and measurement obligations into the baseline and close their concrete contract decisions before `G.BOOT` acceptance. This is not approval of the full product specification or of architecture artifacts outside this review.

Reviewer: delegated independent review agent, 2026-10-18. I did not author or edit the inspected specifications. Scope: the seven completed `docs/quality/*.md` files, cross-checked against `docs/product/accounts.md` and `docs/product/calls.md`. Other authors' unfinished deliverables are excluded. No previous review findings or remediation were supplied for this first round. The file hashes below identify the draft; final commit-bound confirmation remains required by `Q.VER-03`.

All findings are **open**. Proposed owner: quality-specification author, coordinating account/call behavior with the product author. The cases below are specification counterexamples and future acceptance scenarios, not observed native-client failures.

## Findings

### QR1-01 — High: an uncertain vault write has no safe fallback/cancellation outcome

Location: [security.md](../quality/security.md#qsec-03--os-vault-contract), lines 33 and 56–60 (`Q.SEC-03/04`); [accounts.md](../product/accounts.md#auth-004), lines 25–33 (`AUTH-004/005`).

A vault may commit a new credential generation before its completion is lost. The text requires reconciliation **before retrying**, but does not resolve choosing session-only/cancel, a late successful completion, or process exit while that write is uncertain. An older persisted credential can also remain eligible for restoration after a session-only replacement. “Restart requires login” and “no intentional disk persistence” need a defined transition for these cases, beyond withholding a successful-storage badge.

Correction: define unresolved-write handling for every exit from authorization. Suppress automatic restoration with nonsecret account/generation state; reconcile and clean up only the attempted generation without deleting an unrelated/older item. If persistence cannot be ruled out or cleaned up, disclose the residual uncertainty instead of claiming confirmed memory-only storage. Coordinate this with `AUTH-005`'s prohibition on fallback-driven deletion of an older credential.

Acceptance addition: committed-but-unacknowledged write, fallback/cancel followed by late success, crash/restart, and fallback with an existing stored account. Chosen session-only mode must not restore old or new credentials; cancelled/obsolete generations cannot activate. Cancelling reauthorization must not destroy an intentionally retained older persistent session. Unresolved residual storage remains visible until reconciled. These are future adapter tests; no Keychain run is required to correct the specification.

### QR1-02 — High: confirmed account authorization loss does not explicitly terminate live access

Location: [security.md](../quality/security.md#qsec-05--logout-revocation-and-failure-lifecycle), lines 19, 67 and 108 (`Q.SEC-02/05/09`); [accounts.md](../product/accounts.md#auth-007), lines 43–45 (`AUTH-007`); [calls.md](../product/calls.md#call-014), lines 98–100 (`CALL-014`).

Confirmed credential rejection requires a reauthentication state and stopping retries/mutations. Handle invalidation and capture teardown are explicitly required for logout/removal, but not for this transition. An implementation could show “Erneut anmelden” while an already established media/signaling session or an old callback continues operating. A UI-state assertion or rejected new HTTP request would not catch that gap.

Correction: specify account-scoped generation invalidation, cancellation of authenticated operations, local capture/publication teardown, and rejection of late callbacks after **confirmed account** authorization failure. Resume only after reauthorization and fresh policy/session checks. Keep permitted drafts and unrelated accounts intact; distinguish this from a recoverable conversation-session expiry or an action-level permission denial. Local shutdown must not depend on a successful server leave/revocation response.

Acceptance addition: revoke the active account during a call and pending send, then deliver late success and reconnect callbacks. Verify stopped local capture/publication and no automatic authenticated retry before reauthorization; another account continues normally. No claim is made here that app-password revocation automatically tears down an existing remote media session.

### QR1-03 — Medium: ongoing policy freshness has no observable bound

Location: [security.md](../quality/security.md#qsec-10--recording-e2ee-and-changing-permissions), lines 114–120 (`Q.SEC-10`); [verification.md](../quality/verification.md#qver-05--mandatory-cross-cutting-scenarios), line 65 (`Q.VER-05`); [calls.md](../product/calls.md#call-006), `CALL-006/009/014`.

Refreshing on reconnect/join and a changed capability hash does not cover a healthy, long-lived call. “After server changes” and “immediately” lack a detection/freshness contract. The pinned settings explicitly mark `call_end_to_end_encryption` and `grid_videos_limit_enforced` as not changing the hash. A cached allow decision could therefore remain accepted indefinitely unless another trigger occurs.

Correction: require an owner, authoritative refresh sources, explicit unknown/stale states, a bounded freshness policy, and safe behavior when control/policy refresh fails while media transport still works. The bootstrap must choose and justify the bounds and invalidation mechanism; a hash-only conditional refresh is insufficient. State enforcement relative to observation/the agreed bound, without promising instantaneous knowledge of unseen server changes.

Acceptance addition: change a non-hash policy during an uninterrupted call and separately interrupt the policy/control connection while RTP remains available. Require refreshed enforcement or safe suspension within the declared bound. This is a contract requirement now and a real-server/media test later. [Pinned Talk25 settings](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/settings.md)

### QR1-04 — Medium: one-second samples leave a peak/process-accounting loophole

Location: [performance.md](../quality/performance.md#qperf-01--reference-environment-and-accounting), lines 11, 17 and 75 (`Q.PERF-01/02/07`).

The proposed metric correctly includes owned helpers and separates GPU/system costs. However, a short-lived helper or a large allocation released between one-second samples can be absent from the reported footprint peak. Calling it an “observed peak” is honest, but does not make the peak ceiling or complete process coverage meaningful for such work.

Correction: retain the sampled aggregate/p95 metric and require process-lifetime tracking plus a separate peak safeguard. Specify per-process lifetime high-water evidence or a validated equivalent, including short-lived helpers and startup/teardown; distinguish it from simultaneous aggregate samples. Summed lifetime maxima are an upper bound, not a simultaneous peak. Unobserved owned processes must produce incomplete evidence, not a zero or a complete memory pass.

Acceptance addition: qualify the collector using a brief allocation burst and a helper that exits between ordinary sample ticks, documenting overhead and limitations. Apple's API exposes separate `phys_footprint` and `ledger_phys_footprint_peak` fields; availability/collection on the selected Mac remains a later verification step. [Apple XNU task memory fields](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/task_info.h)

### QR1-05 — Medium: workload identity is recorded but insufficiently fixed for budget acceptance

Location: [performance.md](../quality/performance.md#qperf-02--workload-budgets), lines 22–29 and 75 (`Q.PERF-02/07/08`); [test-servers.md](../quality/test-servers.md#qlab-05--reproducibility-evidence), `Q.LAB-05`.

“Continuous paging, scrolling and search” has no prescribed event rates, query distribution or page/cache residency. The conference/screen rows do not unambiguously fix local publishing and screen sender/receiver roles. Recording a chosen seed/codec after a run permits substantially easier workloads under the same budget label.

Correction: require a reviewed, frozen workload manifest before acceptance measurements. It must fix activity cadence/counts, history/cache/page conditions, search mix, fixture/media hashes, local/remote stream directions, screen-sharing role and allowed codec/layer/quality profiles. Keep materially different profiles separate; a lighter selection cannot inherit the same passing claim. Scripts and native runs may be produced later.

Acceptance addition: independent runners reconstruct the same input actions and media load from the manifest; reports verify actual event counts and negotiated quality against it. Divergence is an invalid workload or a separately reviewed profile, not a budget pass.

## Checks and source verification

The direct-accept versus voluntary-prejoin distinction is consistent across `CALL-001/004` and `Q.SEC-09`, including zero capture on ringing. The requirements also preserve network failures versus credential rejection, local logout versus confirmed remote revocation, account/base-path isolation, explicit vault fallback, and separate native/media/performance evidence. These are documented obligations, not implementation results.

Independent read-only checks found **34 unique quality requirement IDs** and **19 valid in-scope relative links/anchors**. Links into unfinished product/compatibility deliverables were not used to manufacture defects.

Primary-source checks on 2026-10-18:

| Assertion checked | Evidence and result |
| --- | --- |
| Login Flow v2 and app-password removal | Official [NC34](https://docs.nextcloud.com/server/34/developer_manual/client_apis/LoginFlow/index.html) and [NC35](https://docs.nextcloud.com/server/35/developer_manual/client_apis/LoginFlow/index.html) manuals confirm external-browser authorization, one-use successful poll response and local removal despite a failed deletion response. The specifications' `latest` URL currently serves NC36 documentation; use the version-specific references for this baseline. |
| Vault status/portability claims | [Apple interaction status](https://developer.apple.com/documentation/security/errsecinteractionnotallowed) and [authentication status](https://developer.apple.com/documentation/security/errsecauthfailed) support the cautious mappings. [CredWriteW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew) uses the current user's credential set. [Secret Service attributes](https://specifications.freedesktop.org/secret-service/latest/lookup-attributes.html) are not secret storage; its [API limits](https://specifications.freedesktop.org/secret-service/latest/ch10.html) support provider qualification. No native adapter was exercised. |
| Consent and policy claims | The pinned [Talk25 call API](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/call.md) documents the consent parameter and rejection without required consent. The [settings table](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/settings.md) confirms the E2EE/HPB association and non-hash-changing settings. These facts support QR1-03, not native E2EE interoperability. |
| Server-family pairing | Pinned [Talk24 manifest](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/appinfo/info.xml) and [Talk25 manifest](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/appinfo/info.xml) match the claimed versions and NC34-only/NC35-only dependency ranges. |
| Image/package pins | The [official image manifest](https://raw.githubusercontent.com/docker-library/official-images/master/library/nextcloud) lists both tags and the stated Docker source revision. Direct HTTPS GETs of [NC34 registry metadata](https://hub.docker.com/v2/namespaces/library/repositories/nextcloud/tags/34.0.5-apache), [NC35 registry metadata](https://hub.docker.com/v2/namespaces/library/repositories/nextcloud/tags/35.0.2-apache), [Talk24 release metadata](https://api.github.com/repos/nextcloud-releases/spreed/releases/tags/v24.0.6) and [Talk25 release metadata](https://api.github.com/repos/nextcloud-releases/spreed/releases/tags/v25.0.6) matched every index/amd64/package digest printed in `Q.LAB-01`. No image or package bytes were downloaded or installed. |
| HPB discovery and Mac measurements | The pinned [installation guide](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/quick-install.md) supports the AIO Talk image-family claim. HPB/TURN pins remain pending as disclosed. Apple API/documentation review supports measurement planning only; it does not qualify a collector or the provisional M2/macOS 15.7 budgets. |

Where the web reader could not retrieve API JSON or Apple's Markdown pages, source checks used read-only HTTPS retrieval and parsing. No server provisioning, app build or functional test was performed.

## Reviewed input SHA-256s

Paths are relative to the repository root. These hashes identify the exact reviewed bytes; no git snapshot existed for this review.

| Input | SHA-256 |
| --- | --- |
| `docs/quality/acceptance-report-template.md` | `b9443339cae0fc3dd6a0c2afa8c3e4a54fb4ce6febe32872d4b4bb7b881e512e` |
| `docs/quality/index.md` | `335e52f98c590554fbe89a37931c0dd64ebe9eb885745155d18aad2b7a8200e5` |
| `docs/quality/performance.md` | `d0b4378b24cf8344dc9caf6ed173902c6a06e5a468d54337f52226d884af5487` |
| `docs/quality/review-report-template.md` | `4698b358c777af16c299e4b7e6fdf350627cde7871f673aca6a0f83d4f5ef8de` |
| `docs/quality/security.md` | `dcb2cbae2f5f57aca62b7a2adb9e21f342cb757e8cf11f98d70595715a66e9c8` |
| `docs/quality/test-servers.md` | `5626118cf742e4f34e1b793a0072803c807a4cd510cd61b1579d3a4b5c9fcf65` |
| `docs/quality/verification.md` | `40f64823751d9515740a7417992443b4967b78983622debbd76baae38f278ba5` |
| `docs/product/accounts.md` | `0b2d79a18966e1733703f9137143b5a36748d6fdbd78ca4fc30b50e9b5a5c0f6` |
| `docs/product/calls.md` | `d5e217440cb30bc545060079f4a916dae1cfa0bdb50d909c831d98155103e1c4` |

## Readiness and limitations

- **Specification consistency:** changes required as above. The lab/review gate structure is suitable as a plan; no false runtime pass was found in the reviewed files.
- **Architecture/bootstrap:** this review supplies required corrections and proof obligations, not an architecture approval. Explicit interfaces must settle credential generations, revocation fan-out, policy freshness and resource measurement before `G.BOOT` can pass. The manager still needs the complete product review, architecture candidate and separately attributable discipline conclusions.
- **Native product/release evidence:** `not run — outside this gate`. macOS Keychain/fallback, capture/permissions, real NC34/35 integration, direct-call race handling, E2EE/media interoperability, collector qualification and performance budgets remain later product/release gates. Missing Mac execution is not a specification defect. HPB/TURN pins are required before relevant lab runs; the monitored private disclosure route must be verified before release.
- Only this report was written. No additional agents were spawned. No specification edits, VM/service startup, git operation, repository creation or publication occurred. No VM teardown was required. Recheck corrections and the final committed candidate before acceptance.

**This conclusion concerns specification/contracts only. Product security, native integration and performance remain unverified.**

