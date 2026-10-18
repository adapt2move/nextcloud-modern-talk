# Independent bootstrap functional/compatibility review — round 1

**Verdict: changes required.** All seven canonical bootstrap commands and the required checker negatives passed their stated assertions on the exact candidate. One fresh bounded backend smoke cycle passed both pinned profiles. These successes do not remove the independently reproduced **BAR1-02** renderer-interface defect or **BFR1-01**, a new architecture-checker false negative. Assign remediation to a different contributor and recheck the resulting committed candidate before accepting this discipline.

**This conclusion concerns bootstrap contracts/scaffolding only. Product security, media integration and native performance remain unverified.** Root PM alone accepts `G.BOOT`; this report authorizes neither publication nor product implementation. All 140 features remain Planned.

## Identity, attribution and scope

- Discipline: function/compatibility plus designated independent executable bootstrap verification; `G.BOOT`, round 1, **2026-10-18**. Reviewer: the independently delegated test sub-agent for this assignment, using Codex. I did not author the candidate, accepted specifications, checkers, lab harness or historical reports. My only repository writes are this report and its [owned evidence directory](evidence/bootstrap-functional-r1/).
- Candidate authors: separate bootstrap contributors, integrator and BI-001 documentation corrector; Git records Maximilian as commit author. This report does not infer separate personal identities for those delegated roles. I used the author evidence as attributed input and independently executed the checks below.
- Brief retained: public `adapt2move/nextcloud-modern-talk`, EUPL-1.2, native Rust/GPUI, macOS primary, no embedded WebView; Talk conversations and presets, both supported server families, capability/configuration/effective-permission gates, direct acceptance without voluntary device-prejoin, zero capture while ringing, OS secure storage with explicit safe fallback, and ordered acceptance gates.
- Required inputs read: AGENTS, IST, product/architecture/quality entry documents, verification/report template, development/lab instructions, [product round 2](product-spec-review-r2.md#mandatory-bootstrap-handoff), [quality round 2](quality-spec-review-r2.md), [design round 2](design-spec-review-r2.md), and integration evidence. The later [architecture round-1 findings](bootstrap-architecture-review-r1.md) supplied a requested additional positive-consumer fixture within this same review round.
- Scope: COMP-001–008; product/feature/CT traceability; AUTH-003–008, CONV-002–005/010, CALL-001–004/006/009–014, MOD-002, CONTENT-006; Q.VER-01–05 and bootstrap/lab boundaries. Architecture, performance and coarse-security discipline conclusions remain their separate reviewers' responsibility. This report supplies executable boundary evidence, not their gate acceptance.

| Identity | Exact value |
| --- | --- |
| Candidate commit | `0b6e94db7faa4bee3d2ae88a5d2da2a5567e05be` |
| Candidate tree | `36133a4336cab291ca174ec9890b9a61c32ce599` |
| Immediate base C2 | `c1cc08ee03ef154c495a20ad0d6c220a1889a3df` |
| Bootstrap source C1 | `d55f39613337f0172b5c209235ae2f43b154a31d` |
| Accepted specification input | `adfb44a2aab2e29ba6574c9dacc09475c534bccf` |
| Specification report continuation / bootstrap base | `d1522594fc16ea884c436e73fe08b0fdabc4a14f` |
| Exact `git archive --format=tar` SHA-256 | `989d3d14c3d411763f821fd3c6eb57cc3743f69162e79017b96dabd538daa4bd` |
| Complete 158-file manifest SHA-256 | `aeb94bc07f3fba32b8024b472af57659ca36d5049c1caa6f19e2fe1d3447c684` |
| `Cargo.lock` SHA-256 | `97bc0a2b02ae77e013d45a47064b61b40dd6c6c6f495be5d245391cee8083cf8` |
| `rust-toolchain.toml` SHA-256 | `f715f440167e20fd8ea9afa0498c3d1ab80174cfcb8cbc895f106a34d907a1e1` |
| Built debug diagnostic SHA-256 | `1fd0aee65e162b09c2aa696005e1d5146bb10f21fefeea167e1833e79a8b7fce` |

[Identity](evidence/bootstrap-functional-r1/identity.json), [manifest](evidence/bootstrap-functional-r1/candidate-manifest.json), [C2 diff](evidence/bootstrap-functional-r1/candidate-vs-c2.diff) and [baseline continuity](evidence/bootstrap-functional-r1/baseline-continuity.json) record the immutable inputs. Tracked input was clean. Concurrent new review outputs were untracked and excluded from the archive. I extracted that archive into a new temporary VM directory, checked every filename, content hash and executable mode before execution, after every canonical command, and at final cleanup. All 158 files matched, with no extras, omissions or changes. Targets and negative/positive fixtures were outside that source directory. No formatter, lockfile repair, source mutation, commit, push or remote creation occurred.

Compared with the 101-file accepted specification continuation, 100 files are byte-identical; the only changed baseline file has precisely the authorized additive `revision-05` anchor. Product, compatibility, quality, historical reviews and visual assets retain their accepted bytes. The prior PR1/QR1/DR findings remain closed at their original specification/design scope; this review does not relabel those historical decisions as runtime evidence.

Execution used the explicitly assigned existing small/CPX22 session, resumed from snapshot 445145085. [Toolchain evidence](evidence/bootstrap-functional-r1/toolchain.json): Ubuntu 26.04.1 LTS, Linux 7.0.0-30-generic x86_64, Python 3.14.4, rustc **1.99.1** (`58cf8f9c35b355199727627d07ae34a4e2846d67`), Cargo 1.99.1, rustfmt 1.10.0-stable and Clippy 0.1.99. `CARGO_HOME` and `RUSTUP_HOME` selected the retained toolchain; HOME/CODEX_HOME were untouched. The lock contains only three local packages and no third-party Rust dependencies. No project build/test/service ran on the host. The diagnostic artifact is a debug build, not a release/native-client artifact or a reproducible-build claim.

## Executed bootstrap verification

[Full commands, stdout/stderr and per-command source equality](evidence/bootstrap-functional-r1/bootstrap-checks.json) were captured by [the independent runner](evidence/bootstrap-functional-r1/verify.py).

| Command / assertion | Result | Actual evidence and limitation |
| --- | --- | --- |
| `python3 tools/check_docs.py` | **Pass** | 56 Markdown files, 636 local references, zero unchecked non-document fragments, **zero errors**. |
| `python3 tools/check_architecture.py` | **Pass for baseline** | Reviewed graph/boundary rules reported satisfied. This is a lexical guard; BFR1-01 limits its detection. |
| `cargo fmt --all -- --check` | **Pass** | Exit 0, no diff/output; no formatting mutation. |
| `cargo check --workspace --all-targets --locked` | **Pass** | All three packages checked; lock unchanged. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | **Pass** | All three packages checked; no warnings. |
| `cargo test --workspace --locked` | **Pass** | **7 contract tests passed**, zero failed/ignored; three crate unit-test targets and two doc-test targets contain zero additional tests. |
| `cargo run --locked -p talk-bootstrap` | **Pass for diagnostic** | Prints contract-bootstrap-only and renderer/network/vault/media/runtime-unconfigured messages and exits. It opens no native client. |

The seven tests are relevant, narrow primitives: account-A cancellation/late work versus account B; action/transport/account rejection classification; request-start-relative lease expiry/deadline; cross-account/runtime/call/sleep-epoch isolation; capability/config/permission/invariant conjunction; exact attempted-generation cleanup/restoration; and UTF-8 byte/page bounds. They do **not** execute a durable journal, provider settlement, actual source refresh, protocol error classifier, actor shutdown, devices, timers or call-policy resolver. They cannot establish the selected deadlines or complete V.AUTH/V.POLICY cases.

The presentation diagnostic discards its snapshot and prints fixed text. To avoid treating that text as service evidence, an [independent temporary probe](evidence/bootstrap-functional-r1/unconfigured-probe.rs) linked the exact compiled candidate crates. [Result](evidence/bootstrap-functional-r1/unconfigured-probe.json): **27 behavioral assertions passed**—four protocol requests, six identity commands, six media-stop reasons, composed availability, one empty/unconfigured initial snapshot, no repeated snapshot, and eight UI intent cases including all three direct-accept media choices. Futures returned `Unconfigured` immediately; intents were rejected. The media publication closure is a no-op because no publication exists; this probe proves no physical device release. It is separate reviewer evidence, not additional repository tests or feature coverage.

### Checker fixtures and BI-001

The [fixture results and exact diffs](evidence/bootstrap-functional-r1/checker-fixtures.json) include clean baselines and restoration. Documentation cases use `--root`; the architecture checker has no such option and was copied with its minimal Cargo/source tree into a temporary fixture so its path-derived root was isolated.

| Isolated case | Actual result | Intended assertion |
| --- | --- | --- |
| Relative, heading, duplicate-heading, reference, encoded/angle filename, HTML ID/link, directory/index and source-line links | Exit 0; 2 Markdown files, 11 references, zero errors | **Pass** |
| Broken-looking fenced, inline, indented code and HTML comments | Included in passing baseline, ignored as examples | **Pass** |
| Missing file | Exit 1, `README.md:26: missing target: 'absent.md'` | **Pass** |
| Missing fragment | Exit 1 with source line and missing-fragment diagnostic | **Pass** |
| Undefined explicit reference | Exit 1 with source line and undefined-reference diagnostic | **Pass** |
| Core → adapter dependency | Exit 1; forbidden/missing dependency edge | **Pass** |
| Target-specific hidden dependency | Exit 1; unreviewed section/hidden platform edge | **Pass** |
| Adapter import into presentation | Exit 1; presentation boundary violation | **Pass** |
| Public renderer `Vec<u64>` field | Exit 1; unbounded renderer type | **Pass** |
| Raw OCS route in core | Exit 1; protocol route leakage | **Pass** |
| Additional public renderer `BTreeMap<u64,u64>` field | **Exit 0**, accepted | **Fail to detect — BFR1-01** |

The first reviewer assertion expected the phrase “missing file”; the actual checker correctly said “missing target.” The [focused recheck](evidence/bootstrap-functional-r1/docs-missing-file-recheck.json) validated that exact failure and restored the passing baseline. Original output is retained. Only reviewer matching was corrected; no candidate checker changed. Each architecture mutation was independently restored and the clean checker rerun. The final exact-candidate docs check again reported 56/636/zero errors.

**BI-001: verified fixed for this candidate.** The historical design report still contains its original `../design/artifacts.md#revision-05` reference. The added stable HTML ID immediately precedes the unchanged historical heading, the candidate checker resolves it, and missing-fragment negatives remain effective. Historical review text and screenshot evidence are unchanged. This closes the link defect, not the whole bootstrap gate.

## Function and compatibility assessment

[Final traceability evidence](evidence/bootstrap-functional-r1/traceability-final.json) identifies **109 requirement/journey/state IDs, 140 unique feature rows all Planned, and 28 unique CT scenarios**. Every feature has a defined requirement link and expanded CT reference; there are no unknown feature references. The first two reviewer parser outputs are retained: their apparent gaps came from omitted `INT` labels and slash/range shorthand, including `STATE-001–007`. Correcting those parsers resolved the apparent gaps without changing a specification. These counts prove trace presence, not completeness of runtime testing. ADR-002 maps current module owners and later slice-specific interfaces; the ledger must gain exact implementation/evidence links as slices ship.

- **Metadata and gates:** COMP-001–008 and ADR-004 retain version-as-evidence rather than version-as-permission. The core `Gate` requires capability/configuration/effective permission/invariant facts and a fresh lease; the complete resolver remains absent. Policy inputs separate account, local/host configuration, room, connected session and signaling provenance. Missing/unqualified sources block affected paths. Both-family mapping, local grid authority and non-hash E2EE/grid refresh are explicit. F/R/D/S/E = 15 s/10 s/3 s/2 s/250 ms and 1 s device release are requirements, not measured guarantees. The [pinned settings source](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/settings.md) was independently checked for the non-hash premise; propagation, cache, sleep and native-enforcement qualification remain unrun.
- **Direct accept versus voluntary prejoin:** CALL-001–004, CT-011–013, ADR-002/004 and `PrepareConference`/`AcceptDirect`/`JoinPrepared` retain the accepted distinction and mandatory OS/consent/security gates. The current application rejects all three paths. No ringing, direct acceptance, permission prompt, prejoin or cancellation race ran in a native client.
- **Conversation/preset restrictions:** PR1-01's final-invariant precedence, all-role channel call denial, announcement leave/default-notification distinction, preservation locks, classified restrictions and `classified_persist` retention remain intact. The fresh lab observes returned preset sets and one channel call-denial invariant; it does not execute the full CT-005/027 restriction matrix or native reconciliation.
- **Re-ring:** MOD-002/F-133/CT-016 still require effective `START_CALL`, eligible active participation and target state, allowing an ordinary member when authorized. Moderator status is not a substitute. Success/no-op and delivery uncertainty remain distinct. **Real re-ring and native behavior not run**; the accepted round-2 source finding is carried forward through identical specification bytes.
- **Poll Close:** CONTENT-006/F-087/CT-010 still require the native author/moderator action, voter denial, state reconciliation and race handling. The backend recheck proves only the owner/author versus ordinary voter subset below. Independent fresh reads of the [Talk24](https://github.com/nextcloud/spreed/blob/0d736588dcfb42becf0e5688e0847e35d5b5efa2/lib/Controller/PollController.php) and [Talk25 released controllers](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/lib/Controller/PollController.php) agree with observed 200 read/vote responses; the [release API prose](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/docs/poll.md) still lists 201. The frozen specification was not edited. Separate moderator/non-moderator-author closure, lost response and concurrent races remain unrun.
- **Identity/bootstrap handoff:** ADR-003 preserves the accepted durable intent, all-generation suppression, deliberate retained-P choice, settlement-before-cleanup, narrow account-loss and independent local-shutdown rules. The later architecture report's BAR1-01 identifies that several of these decisions/lifetimes cannot be expressed by the current `IdentityPort`. I inspected those signatures and retain this outstanding contract objection; I did not execute a missing transition or claim native vault evidence. BAR1-02 has independent executed corroboration below.
- **IST honesty:** the candidate accurately states no GPUI/window, network, vault/database, media, executor, complete resolver, workload runner or collector. All adapters are unconfigured. Selected timings and budgets are unproved requirements. Historical lab and integrator results are labelled and narrow. The integrator should add this review's results and unresolved findings to IST in a new authorized candidate; I did not edit it.

## Independent real backend cycle

One fresh cycle, **16:52:26–16:53:44 UTC**, run `nmtlab-20261018t165226z-c08d0f63`, used the frozen harness unchanged with `sh tests/lab/reprovision.sh --output <temporary-output> --budget-seconds 1800`. [Command evidence](evidence/bootstrap-functional-r1/backend-command.json), [raw sanitized report](evidence/bootstrap-functional-r1/backend-report.json), [commit binding](evidence/bootstrap-functional-r1/backend-binding.json) and [review wrapper](evidence/bootstrap-functional-r1/run_backend.py) are retained. The new report SHA-256 is **`6874334d2fec5fc56ad1bf82ef48496746277c43c00d538bc1263dc26263603f`**.

The historical report remains unchanged at `6b85d61c3a1957b67a30fb710a46f21d96cd2f9dd0d7f43b526df13a4221d84d`. All five historical harness/support hashes match this candidate and the new run; `lab.py` remains `ce5c9eaac74c701795587bbceb67bb885e2b93c74dca02d8004bddda0ca86678`. The frozen harness's literal “uncommitted bootstrap” field is historical text; the companion binding and immutable manifest identify this fresh run's actual input without rewriting the report.

| Pinned/observed profile | HTTP operations | Assertions | Result |
| --- | --- | --- | --- |
| NC34.0.5 (`34.0.5.1`) / Talk24.0.6 | 25 | 96 pass; 1 not applicable because no channel preset returned | **Pass for LAB-001–010; LAB-011 N/A** |
| NC35.0.2 (`35.0.2.2`) / Talk25.0.6 | 30 | 111 pass | **Pass for LAB-001–011** |

The pinned platform image digests were present in the existing cache and reverified. Downloaded Talk archive hashes, installed app versions, NC min=max manifests, signature presence and zero-error integrity checks all matched [pins.json](../../tests/lab/pins.json). Runtime: Docker 29.1.3, PHP 8.5.11, SQLite 3.46.1. Release-source identities are `0d736588dcfb42becf0e5688e0847e35d5b5efa2` and `eed360d176edcb6888dad60965cb694e6bf62ae3`, independently cross-checked against the [24.0.6 release](https://github.com/nextcloud/spreed/releases/tag/v24.0.6) and [25.0.6 release](https://github.com/nextcloud/spreed/releases/tag/v25.0.6). They are distinct from the architecture/specification research pins `17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79` and `f35255ab86f31e8ee894641f6970a09ce4b90927`; branch research was not silently substituted for released-profile evidence.

Observed positive/negative scope: four distinct Basic-auth synthetic identities; anonymous room-list HTTP 401/OCS 997; capabilities and role-dependent preset discovery; owner/member private group joins with room sessions; Unicode message/author identity; authenticated outsider room/history 404 and no list leakage; poll creation 201, member vote 200, voter close 403 with open state retained, owner/author close 200, closed result readable at 200 and late vote 400. R35's ordinary-group call-control join/leave returned 200 before channel-owner call denial 403. Calls used `flags=1`, with no WebRTC peer, capture or media exchange. Both profiles advertised calls/conversation creation enabled, recording disabled, consent 0. Preset differences were observations of these fixtures, not a runtime version switch.

These are **partial CT-001/002/004–007/010 observations**, never whole CT passes or native feature completion. No live capability/configuration/permission mutation, broad preset matrix, account revocation, public guest flow, TLS, browser Login Flow/app-password issuance, federation, HPB/TURN, E2EE or native client was exercised. The narrow Basic-auth/API probe does not qualify ADR-003's actual app-password rejection classifier.

## Findings and required remediation

### BFR1-01 — Medium: direct unbounded map field escapes the renderer guard

**State: open. Owner: separately delegated architecture/governance contributor.** Affects ADR-002 renderer/resource boundary, Q.PERF-05 and Q.VER-01/03. Location: [check_architecture.py](../../tools/check_architecture.py), lines 101–107, especially line 102.

Reproduction: in the isolated architecture fixture append `pub struct FixtureUnboundedMap { pub rows: std::collections::BTreeMap<u64, u64> }` to `crates/talk-core/src/ui.rs`, then run `python3 tools/check_architecture.py`. Expected: reject the exposed unbounded renderer container. Actual: exit **0** and the normal success message. The otherwise equivalent `Vec<u64>` case exits 1. Exact diffs/results are in `exploratory-unbounded-map-field` in [checker-fixtures.json](evidence/bootstrap-functional-r1/checker-fixtures.json); [verify.py](evidence/bootstrap-functional-r1/verify.py) reproduces both and restores the fixture.

This is a detector defect, not an existing unbounded map in the candidate or an observed performance failure. The current UI uses bounded values. The checker honestly disclaims being a Rust parser/security proof, but this directly named standard collection demonstrates that its public-field guard cannot support an unqualified unbounded-container claim. No alias or complicated Rust construct was needed.

Required correction: make the reviewed presentation data surface and guard consistent, including direct standard unbounded collections, and document any remaining lexical limitations precisely. Preserve dependency, secret, worker/frame and raw-route checks. Add this negative alongside a valid bounded positive consumer; do not broadly disable restrictions to repair BAR1-02. Acceptance: the current baseline and valid bounded consumer pass, this map case and the five required architecture negatives fail with the intended diagnostic, and pinned checks pass on the new exact candidate. No fix or exception is supplied by this reviewer; author response/remediation commit: none.

### BAR1-02 — Medium: independently reproduced positive-consumer failure

This retains the architecture reviewer's ID/attribution. **State: open; executed corroboration, not a new review round.** Locations: [ui.rs](../../crates/talk-core/src/ui.rs), lines 4–7, 24–43 and intent fields; checker lines 101–110. Owner: separate architecture/governance contributor.

[Reproducer](evidence/bootstrap-functional-r1/renderer-consumer.py) and [full compiler/checker evidence](evidence/bootstrap-functional-r1/renderer-consumer.json) replace only the temporary copy's presentation consumer. It matches all `Decision`/`PersistenceState` cases and constructs typed logout/select/direct-accept intents.

1. Importing `AccountId`, `CallScope`, `ConversationId`, `Decision` and `PersistenceState` through allowed `talk_core::ui` passes the architecture checker but `cargo check --workspace --all-targets --locked` exits **101**, with **five E0603 private-type errors**.
2. Importing the types directly from `ids`, `policy` and `identity` compiles successfully, but the architecture checker exits **1** with its presentation-boundary diagnostic.

The lockfile remains unchanged; the original fixture is restored after each case and its checker passes. The candidate source itself was never edited. This reproduces the inconsistency hidden by the diagnostic's discarded snapshot. Expected: an ordinary pure presentation consumer can name/match the public data and construct typed intents through the approved surface. Required acceptance: that positive consumer must pass both compiler and guard while all forbidden-edge/container/route fixtures continue to fail. An import-spelling bypass is not a correction. No native/GPUI implementation is needed to close this contract defect.

**BAR1-01 remains an attributed outstanding high-severity architecture finding.** Its omitted explicit session-only/cancel-retain transitions and pre/post-runtime authority contradiction require the separate contributor's typed-contract remediation and focused future contract tests. Actual journal/vault/account-loss cases remain not run here; no credential leak is alleged.

## Cleanup, remaining evidence and handoff

The lab ran with synthetic credentials, separate internal Docker bridges, **zero published host ports or forwards**, resource limits, readonly administrator secret-file mounts and user-password stdin. `/run` was confirmed tmpfs. The harness removes only exact run-labelled objects; no broad prune was used. Both profiles' two containers, two networks and two volumes were deleted, along with runtime secret directories. Independent before/after/final inspections found **zero lab-labelled containers/networks/volumes and zero lab runtime directories**. [Cleanup evidence](evidence/bootstrap-functional-r1/cleanup.json) records deletion of my temporary targets, fixtures, source archive extraction and probe executable. The safe Rust toolchain/public image caches are retained. Logical deletion is not a forensic-erasure claim; external-host reachability was not probed.

[Final source equality](evidence/bootstrap-functional-r1/source-final.json) still proves 158/158 unchanged files. [Final transfer](evidence/bootstrap-functional-r1/final-transfer.json) records explicit export and per-file hash verification of the sanitized VM evidence; automatic source upload was not treated as download. [Lifecycle evidence](evidence/bootstrap-functional-r1/lifecycle.json) confirms the same session is **hibernated**, snapshot **445145409**, with no server present. Cumulative parent-session VM time is 2,526.95 seconds; Hetzner estimates are **EUR 0.09614 net** total and **EUR 0.24166 net/month** for retained session snapshots, with no reported warning. These are cumulative estimates, not incremental review charges; traffic, shared base-image costs and historical price changes are excluded. A nonsecret operational handoff is also saved at the requested verifier-status location outside the repository.

**Not run:** GitHub CI itself; native macOS GPUI/accessibility/Keychain/permissions; browser login/2FA/SSO and app-password lifecycle; complete account-loss/uncertain-vault/cancellation/recovery cases; actual direct/prejoin/ringing/media; policy changes under live media; native E2EE, internal/HPB/TURN interoperability; complete preset/re-ring/poll-race/guest/file/federation scenarios; frozen native workloads, actual delivered-profile validation, complete-lifetime/peak collector qualification and performance budgets. No sample/peak/startup/CPU/energy result is claimed. Missing future adapters are expected; the findings concern present bootstrap contracts/checkers.

Recommendation: **do not accept the functional/executable-bootstrap portion of G.BOOT yet**. Carry forward the successful exact-candidate checks, bounded backend results and BI-001 closure as scoped evidence. Consolidate BFR1-01 and the executed BAR1-02 corroboration with BAR1-01 under separate implementation ownership; obtain a committed remediation candidate, updated ADR/IST and focused independent recheck. No finding is silently waived. No product feature status change or publication approval is warranted. Reviewer confirms this conclusion for the full candidate SHA above, round 1, 2026-10-18.
