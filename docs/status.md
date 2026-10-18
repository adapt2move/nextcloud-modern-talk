# Current state — IST

Last integrated: **2026-10-18**. This records the independently reviewed bootstrap and its authorized public work-in-progress publication. **All four round-1 review disciplines require changes; G.BOOT is not accepted.** Corrections are in progress under separate contributor ownership; independent recheck and PM acceptance remain pending. Documentation/publication-only change: **no product feature status change; all 140 remain Planned**.

[Integration round 1](reviews/bootstrap-integration-r1.md) binds the earlier source commit `d55f39613337f0172b5c209235ae2f43b154a31d` to the integrator's rerun, five backend harness hashes, sanitized command evidence and original BI-001 finding. Its historical failures and evidence remain unchanged. The later independent reviews below all identify exact source **`0b6e94db7faa4bee3d2ae88a5d2da2a5567e05be`**; their conclusions do not claim to review a future remediation candidate. This publication adds their reports/evidence and updates README, SECURITY and this status, without changing reviewed Rust, checkers, ADRs, fixtures or historical reports.

## Gate state

| Gate | State and evidence |
| --- | --- |
| `G.SPEC` | PM accepted the independently passed specification input `adfb44a2aab2e29ba6574c9dacc09475c534bccf`, with report-only continuation `d1522594fc16ea884c436e73fe08b0fdabc4a14f`. [Product round 2](reviews/product-spec-review-r2.md) carries the accepted quality/design evidence. |
| `G.BOOT` | **Changes required; not accepted.** [Architecture](reviews/bootstrap-architecture-review-r1.md), [function/compatibility](reviews/bootstrap-functional-review-r1.md), [performance](reviews/bootstrap-performance-review-r1.md) and [coarse security](reviews/bootstrap-security-review-r1.md) all require corrections. Independent executable verification passed the seven canonical commands and required checker negatives on `0b6e94d…`; BI-001 is verified fixed. Open findings below, remediation-candidate verification, affected independent rechecks and PM acceptance remain pending. |
| `G.PRODUCT`, `G.RELEASE` | Not reached. Full implementation waits for PM acceptance of `G.BOOT`. |

## Present versus absent

| Area | Actual state |
| --- | --- |
| Product/design | Accepted written specifications, 140 Planned feature entries, compatibility matrix, primary-source citations, reviewed HTML prototypes and bitmap design assets. These are design/reference artifacts. |
| Workspace | Three packages, pinned Rust 1.99.1 with rustfmt/Clippy, path-only lockfile, no third-party Rust dependencies. The authored lockfile passed pinned Cargo's locked checks without regeneration or byte changes. |
| Core | Typed local IDs/scopes and conversation kinds; bounded text/pages and renderer snapshots/intents; normalized policy-source bundle; pure admission/cancellation, minimum gate/freshness algebra and attempted-generation cleanup checks; worker service interfaces. These checks are only contract primitives, not complete authorization/policy services. |
| Composition | Headless diagnostic composes unconfigured protocol/identity/media slots. All service requests return `Unconfigured`; user intents are rejected. No account or device is opened. |
| Renderer | **Absent.** GPUI is selected, but is not a dependency and no native window, accessibility integration or WebView exists. HTML assets are not embedded. |
| Network | **Absent.** No HTTP/TLS/DAV/signaling adapter, authenticated request, server login, conversation, chat or transfer implementation. Protocol routes are documented only. |
| Backend lab | The historical [released-backend smoke report](testing/backend-smoke-2026-10-18.md), failed attempts and cleanup remain unchanged. The later [independent functional review](reviews/bootstrap-functional-review-r1.md#independent-real-backend-cycle) ran one fresh cycle on both pinned profiles with the unchanged harness: R34 **96 assertions passed + 1 N/A**; R35 **111 passed**. The fresh report and exact-source binding are linked below. These are bounded API observations, not native-client or complete CT passes. |
| Identity/storage | **Absent.** No Keychain/other vault, secret buffer, Login Flow, metadata database, crash recovery or session-only login implementation. Contracts specify their future behavior. |
| Media/platform | **Absent.** No media engine, capture, permissions adapter, media worker, E2EE, frame transport, clock adapter or enforcement watchdog. Numeric deadlines are acceptance requirements, not measured guarantees. |
| Runtime | **Absent.** No executor, account actor, queue, scheduler, disk cache or shutdown supervisor. Bounded queues/caches are design limits; only boundary text/page bounds and a coalesced initial snapshot exist in code. |
| Workloads/collectors | [Interface and qualification plan](architecture/measurement.md) authored; no native workload runner, stream validator, complete-lifetime collector or benchmark result supplied by this author. Lab/testing outputs are separately owned and must carry their own evidence. |

## Independent round-1 results and open corrections

The [functional reviewer](reviews/bootstrap-functional-review-r1.md#executed-bootstrap-verification) independently executed all seven [canonical commands](architecture/README.md#validation) on the exact `0b6e94db7faa4bee3d2ae88a5d2da2a5567e05be` candidate using the assigned Linux development VM and pinned Rust 1.99.1. Documentation and architecture checks, formatting, locked compile, Clippy with warnings denied, **seven contract tests** and the unconfigured diagnostic passed. The docs check reported 56 Markdown files, 636 local references and zero errors. Crate unit-test and doc-test targets contain zero additional tests. A separate reviewer probe passed **27 Unconfigured behavioral assertions**; it is independent evidence, not 27 new repository tests or native feature coverage.

The required documentation positives/negatives and five architecture negatives passed their intended assertions. The later exploratory `BTreeMap` negative was incorrectly accepted, producing BFR1-01; the baseline checker pass does not close that finding. A positive renderer consumer independently reproduced BAR1-02: imports through the allowed UI surface fail compilation, while direct domain imports compile but fail the guard. [Checker evidence](reviews/evidence/bootstrap-functional-r1/checker-fixtures.json) and [renderer-consumer evidence](reviews/evidence/bootstrap-functional-r1/renderer-consumer.json) preserve the exact outcomes and reviewer assertion corrections.

**BI-001 is verified fixed** by the independent exact-candidate docs run and missing-fragment negatives. The original report and reference remain unchanged; the additive `revision-05` HTML ID resolves the link. This closes only the link finding.

The independent backend run used NC34.0.5/Talk24.0.6 and NC35.0.2/Talk25.0.6: R34 recorded 25 HTTP operations, **96 passing assertions and one not applicable** channel-preset case; R35 recorded 30 operations and **111 passing assertions**. The [fresh backend report](reviews/evidence/bootstrap-functional-r1/backend-report.json) SHA-256 is **`6874334d2fec5fc56ad1bf82ef48496746277c43c00d538bc1263dc26263603f`**. Its [binding](reviews/evidence/bootstrap-functional-r1/backend-binding.json) ties all five unchanged harness/support hashes to the reviewed candidate. The historical report remains `6b85d61c3a1957b67a30fb710a46f21d96cd2f9dd0d7f43b526df13a4221d84d`. The fresh report retains the frozen harness's historical source-label text; the companion binding supplies its actual source identity. Synthetic API checks do not qualify native media, Login Flow/app passwords, full CT scenarios or performance. The verifier recorded zero remaining lab resources/runtime directories and hibernation at snapshot 445145409; publication did not resume that VM.

| Open finding | Required correction and ownership |
| --- | --- |
| **BAR1-01 / BSR1-01 — high, same identity seam** | Separate identity/architecture contributor: explicit session-only/cancel-and-retain decisions and valid pre-runtime/recovery/post-cancellation authority; preserve durable disposition and exact-generation cleanup. Both independently attributed findings remain open. |
| **BAR1-02 — medium, UI surface** | Separate architecture/governance contributor: a complete permitted presentation data surface and consistent guard, with a compiling bounded positive consumer. |
| **BFR1-01 — medium, unbounded `BTreeMap` guard** | Same boundary/guard correction owner: reject directly exposed standard unbounded collections while preserving required negatives and documenting lexical limits. |
| **BSR1-02 — medium, pinned native-engine decision** | Separate media/platform and architecture contributor: the required source/artifact-pinned engine and FFI/ownership decision, or an explicit reviewed PM scope disposition. Native integration remains later work. |
| **BPR1-01 — medium, media allocation bounds** | Separate media/platform and architecture contributor: finite frame/audio backing and aggregate pool bounds, checked pre-allocation admission, ownership and reclamation. No native measurements are claimed. |

Corrections are in progress outside this publication candidate. No remediation is applied or finding waived here. Independent recheck of the resulting committed candidate and PM acceptance remain pending; full product implementation remains gated.

## Historical integration results

The original architecture author did not execute builds or tests. The integrator resumed the existing small development VM and ran the [canonical checks](architecture/README.md#validation) on Ubuntu 26.04.1 LTS, Linux x86_64, Python 3.14.4 and the exact pinned Rust 1.99.1 toolchain. Architecture, locked compile, Clippy with warnings denied, seven contract tests and the unconfigured diagnostic passed. The crate unit-test and documentation-test targets contain zero additional tests. These earlier results do not qualify an adapter or feature; the integrator did not run GitHub CI.

The initial format check failed in `crates/talk-core/src/ui.rs` and `crates/talk-core/tests/contracts.rs`. Explicit VM `cargo fmt --all` changed layout/trailing commas only; the resulting files were copied back and hash-verified locally, and the format recheck passed. The first retrieval attempt encountered an automatic source upload that restored the unformatted local bytes; formatting and export were repeated within one VM job. No interface or behavior was redesigned. The lockfile needed no correction. Exact committed-source reruns and artifact identities belong in the integration report; these are author/integration evidence only.

**Integration finding BI-001 — originally fixed pending verification, now independently verified fixed above:** The initial `python3 tools/check_docs.py` scan exited 1 (55 Markdown files, 629 local references, one error): [historical design review](reviews/design-spec-review.md), line 46, links to `../design/artifacts.md#revision-05`, while the target heading is “Revision 05 — historical”. The PM-authorized correction adds `<a id="revision-05"></a>` immediately before that unchanged heading in `docs/design/artifacts.md`. Historical review texts and evidence, screenshots and accepted product/design semantics remain unchanged; the checker was not weakened or bypassed.

The anchor-correction author used static source inspection to confirm that the existing checker recognizes explicit HTML `id` anchors and ran no project checks, negative fixtures, builds, services or VM lifecycle operations. That author's pending-verification statement is superseded only by the separately attributed independent run above. `G.BOOT` remains unaccepted; **no product feature status change; all 140 remain Planned**.

Before accepting this bootstrap: resolve the open review findings, independently verify the remediation candidate and affected positive/negative fixtures, confirm exact candidate/artifact/toolchain identities, obtain affected independent discipline rechecks and PM acceptance. The round-1 reviews and executed fixtures are now complete for `0b6e94d…`; no author self-approval is recorded.

## Public work in progress

The user authorized immediate public publication at [adapt2move/nextcloud-modern-talk](https://github.com/adapt2move/nextcloud-modern-talk) under EUPL-1.2 while corrections continue. GitHub repository setup confirmed public visibility and enabled private vulnerability reporting on 2026-10-18; [SECURITY](../SECURITY.md) records the verified setting without claiming monitoring or a response SLA. This is source/documentation publication, not a release, usable-app claim or gate acceptance.

The publication contributor statically inspected the intended history and all new review evidence for public hygiene and preserved the reports and evidence identities. No host project tests, builds, services or VM resume were performed for publication. Previously executed checks remain attributed to their exact reviewed source; GitHub CI on the published commit is separate evidence and must be checked before claiming a CI pass.

## Remaining native qualification

Before a native slice: qualify the bounded policy sources for each server/signaling deployment; implement and fault-test credential durability; select/pin/review GPUI and native media dependencies and licenses; implement the runtime, media watchdog and native clock; secure macOS hardware and collector qualification. Unqualified policy paths remain blocked. Native vault/capture/renderer/performance evidence cannot be obtained from the Linux lab. See [open qualification boundaries](architecture/README.md#qualification-boundaries).
