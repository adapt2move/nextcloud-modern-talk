# Bootstrap integration round 1 — preliminary evidence

**2026-10-18; delegated bootstrap integrator, not an independent gate reviewer.** The candidate is committed for review, with **BI-001 open**. This report does not approve `G.BOOT`. All 140 product features remain Planned; **no product feature status change**. [IST](../status.md) records actual scope and remaining evidence.

## Candidate identity

- Accepted specification: `adfb44a2aab2e29ba6574c9dacc09475c534bccf`; report-only base: `d1522594fc16ea884c436e73fe08b0fdabc4a14f`.
- **C1 source commit:** `d55f39613337f0172b5c209235ae2f43b154a31d`; tree `5f3e3e401079243832652889e626174f0bf0db9c`. The local working tree was clean before the committed-source run.
- C1 adds 55 authored bootstrap files. The accepted product, compatibility, quality, design, assets, licenses/notices and all historical reviews remain byte-identical to the base.
- All **156** C1 file hashes matched the VM before and after execution, with no additional, missing or stale source files. Sorted content-manifest SHA-256: `396a9e4a4359dacd410a59a1604eac1cfc188bb32d3d4f2ee38dc7e4b56a104f`. [Machine-readable evidence](evidence/bootstrap-integration-r1.json) contains the complete manifest, encoding, commands, outputs and failed attempts.
- **C2 is report-only:** it adds this report and its sanitized JSON evidence and updates the IST report link. It changes no C1 Rust, toolchain, checker, architecture contract, harness or historical evidence. Independent reviewers must name final HEAD and inspect C1→C2, rather than treating this report as final-candidate approval.

## Execution and corrections

All project commands ran through the existing parent agent-dev-box session on small CPX22, **Ubuntu 26.04.1 LTS, Linux 7.0.0-30-generic x86_64, Python 3.14.4**. No host build/test or fixture service was used. Toolchain installation followed the [official Rust distribution](https://rust-lang.org/tools/install/): the HTTPS rustup installer matched its official sibling SHA-256 before execution. The first installation setup stopped because the remote shell had no `HOME`; explicit Cargo/rustup storage directories resolved that environment issue without selecting another compiler.

Executed versions: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, full compiler commit `58cf8f9c35b355199727627d07ae34a4e2846d67`, host `x86_64-unknown-linux-gnu`, LLVM 23.1.1; Cargo `1.99.1 (5f94df478 2026-08-27)`, rustfmt `1.10.0-stable`, Clippy `0.1.99`. Cargo, rustc, rust-std, rustfmt and Clippy were installed for that exact toolchain. No registry dependencies were introduced.

The first `cargo fmt --all -- --check` failed. Explicit `cargo fmt --all` changed only layout/trailing commas in `crates/talk-core/src/ui.rs` and `crates/talk-core/tests/contracts.rs`. An automatic source upload restored the old bytes before a separate retrieval attempt; the local hash guard rejected the transfer and a format recheck still failed. Repeating formatting and exporting the changed files within the same VM job allowed explicit download and hash-verified local application. The exact formatting diff is retained in the JSON; SHA-256 `5fecb4e8a8d2241eb00b7600e005a10aa9c059c1008576e66c98ce6ce065de94`. No interface, behavior, requirement or checker was changed to obtain a pass.

The authored `Cargo.lock` passed unchanged; `cargo generate-lockfile --offline` was **not needed or run**. Lockfile SHA-256: `97bc0a2b02ae77e013d45a47064b61b40dd6c6c6f495be5d245391cee8083cf8`. Toolchain-file SHA-256: `f715f440167e20fd8ea9afa0498c3d1ab80174cfcb8cbc895f106a34d907a1e1`.

## C1 committed-source results

Rerun window: **16:31:56–16:31:57 UTC**. Paths in durable output are replaced with `<repo>`, `<target>`, `<toolchain>` and `<installer>`; exit codes and command output are otherwise retained.

| Exact command from repository root | Result |
| --- | --- |
| `python3 tools/check_docs.py` | **Fail, exit 1:** 55 Markdown files, 629 local references, 0 unchecked non-document fragments, 1 error: BI-001. |
| `python3 tools/check_architecture.py` | Pass, exit 0: reviewed bootstrap dependency and boundary rules hold. |
| `cargo fmt --all -- --check` | Pass, exit 0, after the explicit correction. |
| `cargo check --workspace --all-targets --locked` | Pass, exit 0, all three packages. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass, exit 0. |
| `cargo test --workspace --locked` | Pass, exit 0: **7 contract tests, 0 failures**; three crate targets and two documentation-test targets have 0 additional tests. |
| `cargo run --locked -p talk-bootstrap` | Pass, exit 0: prints the contract-only and unconfigured renderer/network/vault/media/runtime state. |

The debug diagnostic artifact SHA-256 was `d39468f4b212f28e3a45089c8771a734a95742ad750008b5f138badb85f7e84f`. This identifies an unconfigured composition diagnostic, not a native application, release artifact or performance measurement. Build output was kept outside the public source tree.

## BI-001 — open documentation-check blocker

**Owner:** PM to assign a separately authorized correction; independent verifier to recheck. **Scope:** Q.VER-03/07 and accepted-baseline/history preservation. This blocks a complete canonical-check pass, not compilation.

Reproduction: run `python3 tools/check_docs.py`. Actual output:

```text
docs/reviews/design-spec-review.md:46: missing fragment: '../design/artifacts.md#revision-05'
Documentation: 55 Markdown files, 629 local references, 0 non-document fragments unchecked, 1 errors.
```

The [historical review](design-spec-review.md), line 46, retains the earlier fragment. The preserved [artifact document](../design/artifacts.md) now uses heading “Revision 05 — historical” and has no explicit `revision-05` anchor. Both bytes already existed in the accepted base. Expected: preserved review references resolve. The integrator left both files unchanged because this brief explicitly preserves accepted design documents and historical reviews. No exclusion, alias heuristic or checker weakening was introduced. The PM must resolve that bounded scope conflict and obtain a fresh check; this report does not silently accept the failure.

## Binding the existing backend evidence

The [successful backend report](../testing/backend-smoke-2026-10-18.md) and [JSON](../testing/evidence/backend-smoke.json) were **not edited or rerun**. Report SHA-256 remains `6b85d61c3a1957b67a30fb710a46f21d96cd2f9dd0d7f43b526df13a4221d84d`. Every recorded executed harness/support hash matches C1:

| File | SHA-256 |
| --- | --- |
| `tests/lab/README.md` | `52219b128c0396711ed106140f1eac11a8a3422056e5aff265dd5d3a6d28ee00` |
| `tests/lab/cleanup.sh` | `90dc99b45324e072ee3fb733b490e7f9d8bbc2b9621da84f38c7b574ab02e132` |
| `tests/lab/lab.py` | `ce5c9eaac74c701795587bbceb67bb885e2b93c74dca02d8004bddda0ca86678` |
| `tests/lab/pins.json` | `fc2bba4225c1eecab7b4faec51353f9170e5c249999920254a6413f939399016` |
| `tests/lab/reprovision.sh` | `c826fe3a6d2193aa07ae00bd8daf06f13fb71e5b16e38bced7e97d8d4e4dbb36` |

The earlier run remains bounded to NC34.0.5/Talk24.0.6 (25 HTTP operations; 96 assertions pass, 1 not applicable) and NC35.0.2/Talk25.0.6 (30 operations; 111 assertions pass). Failed attempts, cleanup and the observed poll vote/read 200 versus API-prose 201 discrepancy remain intact. Hash binding does not extend those observations or independently approve them.

## Handoff and remaining scope

Static staged-tree inspection found no matching credential/private-path patterns, secret-bearing fixtures, runtime output, symlinks or accidental parent research/design copies. EUPL-1.2 and Manrope notices remain intact. Only intended child-repository artifacts were staged. No remote repository, push or publication was performed.

**Not run:** independent checker negative fixtures, GitHub CI execution, another backend run, full CT scenarios, native login/GPUI/vault/capture/accessibility/WebRTC/media and performance. These preliminary contract checks do not establish native behavior or complete any feature. The next owners are the PM and separately delegated architecture/module, function/compatibility, performance and coarse-security reviewers. They must resolve BI-001, review exact final HEAD, run independent verification and obtain PM acceptance before product implementation.
