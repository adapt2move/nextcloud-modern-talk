# Current state — IST

Last integrated: **2026-10-18**. This records authored bootstrap scope and preliminary integrator execution, not an independent approval or a release claim. Infrastructure-only change: **no product feature status change; all 140 remain Planned**.

[Integration round 1](reviews/bootstrap-integration-r1.md) binds source commit `d55f39613337f0172b5c209235ae2f43b154a31d` to the committed-source rerun, all five backend harness hashes, sanitized command evidence and open finding BI-001. Its C2 continuation changes only reports/evidence and this IST link; independent reviewers must inspect final HEAD.

## Gate state

| Gate | State and evidence |
| --- | --- |
| `G.SPEC` | PM accepted the independently passed specification input `adfb44a2aab2e29ba6574c9dacc09475c534bccf`, with report-only continuation `d1522594fc16ea884c436e73fe08b0fdabc4a14f`. [Product round 2](reviews/product-spec-review-r2.md) carries the accepted quality/design evidence. |
| `G.BOOT` | **Pending; not approved.** Preliminary integration checks ran on the assigned development VM. Rust/architecture checks passed after explicit formatting; the documentation check has the open historical-link failure below. Independent checker fixtures, all four reviews, final-candidate confirmation and PM acceptance remain pending. |
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
| Backend lab | The separately authored [released-backend smoke report](testing/backend-smoke-2026-10-18.md) records a bounded API pass for NC34.0.5/Talk24.0.6 and NC35.0.2/Talk25.0.6, with failed attempts and cleanup retained. Integration confirmed all five recorded harness/support hashes and the successful report checksum. No backend rerun occurred; independent review remains pending. This is not a native-client or complete CT pass. |
| Identity/storage | **Absent.** No Keychain/other vault, secret buffer, Login Flow, metadata database, crash recovery or session-only login implementation. Contracts specify their future behavior. |
| Media/platform | **Absent.** No media engine, capture, permissions adapter, media worker, E2EE, frame transport, clock adapter or enforcement watchdog. Numeric deadlines are acceptance requirements, not measured guarantees. |
| Runtime | **Absent.** No executor, account actor, queue, scheduler, disk cache or shutdown supervisor. Bounded queues/caches are design limits; only boundary text/page bounds and a coalesced initial snapshot exist in code. |
| Workloads/collectors | [Interface and qualification plan](architecture/measurement.md) authored; no native workload runner, stream validator, complete-lifetime collector or benchmark result supplied by this author. Lab/testing outputs are separately owned and must carry their own evidence. |

## Validation and remaining decisions

The original architecture author did not execute builds or tests. The integrator resumed the existing small development VM and ran the [canonical checks](architecture/README.md#validation) on Ubuntu 26.04.1 LTS, Linux x86_64, Python 3.14.4 and the exact pinned Rust 1.99.1 toolchain. Architecture, locked compile, Clippy with warnings denied, seven contract tests and the unconfigured diagnostic passed. The crate unit-test and documentation-test targets contain zero additional tests. These results do not qualify an adapter or feature, and the GitHub CI workflow itself was not run.

The initial format check failed in `crates/talk-core/src/ui.rs` and `crates/talk-core/tests/contracts.rs`. Explicit VM `cargo fmt --all` changed layout/trailing commas only; the resulting files were copied back and hash-verified locally, and the format recheck passed. The first retrieval attempt encountered an automatic source upload that restored the unformatted local bytes; formatting and export were repeated within one VM job. No interface or behavior was redesigned. The lockfile needed no correction. Exact committed-source reruns and artifact identities belong in the integration report; these are author/integration evidence only.

**Open integration finding BI-001:** `python3 tools/check_docs.py` exits 1 because [historical design review](reviews/design-spec-review.md), line 46, links to `../design/artifacts.md#revision-05`, while the preserved target now has heading “Revision 05 — historical” and no `revision-05` anchor. The initial scan reported 55 Markdown files, 629 local references and this one error. Both files predate the bootstrap. They remain unchanged under the accepted-baseline/history preservation requirement; the checker was not weakened or bypassed. PM must assign a separately authorized correction and independent recheck before claiming a complete bootstrap-check pass.

Before accepting this bootstrap: resolve BI-001 and review findings, independently rerun the canonical checks and checker negative fixtures, confirm exact candidate/artifact/toolchain identities, complete architecture/module, function/compatibility, performance and coarse-security reviews, and obtain PM acceptance. Negative fixtures were deliberately not run by the integrator. The formatting correction's resulting bytes still need independent review. No author self-approval is recorded.

Before a native slice: qualify the bounded policy sources for each server/signaling deployment; implement and fault-test credential durability; select/pin/review GPUI and native media dependencies and licenses; implement the runtime, media watchdog and native clock; secure macOS hardware and collector qualification. Unqualified policy paths remain blocked. Native vault/capture/renderer/performance evidence cannot be obtained from the Linux lab. See [open qualification boundaries](architecture/README.md#qualification-boundaries).
