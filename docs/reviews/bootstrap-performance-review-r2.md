# Independent bootstrap performance review — round 2

**Verdict: pass for the stated bootstrap contracts/scaffolding scope. BPR1-01 (medium) is verified fixed at resource-contract level on the exact candidate below. No new performance blocker or exception is proposed.** The finite allocation choices can now be independently derived; enforcing them in native code and satisfying the accepted performance budgets remain unproved.

**This conclusion concerns bootstrap contracts/scaffolding only. Product security, media integration and native performance remain unverified.** It does not accept G.BOOT, authorize a release or establish a RAM/CPU/power/latency result. Root PM acceptance is separate; all 140 features remain Planned.

## Identity, independence and scope

- Discipline/gate/date: **performance, G.BOOT, independent round 2, 2026-10-18**. Reviewer: the separately assigned Codex security/performance review agent in this new task. I did not author the inspected implementation, specifications, media decisions, checkers, fixtures or remediation. This is not a continuation or rewrite of round 1. The [security conclusion](bootstrap-security-review-r2.md) is separate.
- Candidate authors: separately delegated identity/UI and media proposal contributors plus the remediation integrator, as attributed in [the author handoff](bootstrap-remediation-r1.md) and its immutable provenance. Author and CI execution are not my execution.
- Original brief retained: public EUPL-1.2 native Rust/GPUI client, macOS first, no WebView; Buzz-inspired Manrope/blue design; unified Talk conversations with chat/files/calls, 1:1/groups/guests and presets/effective permissions; both NC34/Talk24 and NC35/Talk25 through capabilities/configuration/permissions; direct acceptance without voluntary device-prejoin, zero capture while ringing and mandatory safety gates; OS-vault credentials with explicit safe fallback; high performance, Deep Modules, actual-state documentation and ordered gates before full implementation.
- Read AGENTS, IST, product/quality/architecture entrypoints, verification/report template, [mandatory product handoff](product-spec-review-r2.md#mandatory-bootstrap-handoff), [quality](quality-spec-review-r2.md) and [design](design-spec-review-r2.md) round 2 with relevant prior findings/remediations, all four bootstrap round-1 reports and the consolidated remediation. Scope: Q.PERF-01–08, Q.SEC-10 control costs, Q.VER-01–03/07/08, V.PERF.PEAK/WORKLOAD; finite native/non-media resource contracts, lifetime/reclamation, workload/collector continuity and truthful evidence. Architecture and function verdicts are separate.

| Identity | Exact value |
| --- | --- |
| Final candidate | `54cde8e317ed0ea98d88255b9f12cb9a1151229b` |
| Candidate tree | `ccde84f7a2ab850f5aaa2ff4fb0e4d76c7e3f115` |
| Public integration base | `68e6f5299bae9deb234ed1ef0ab3976878796afe` |
| Source-changing parent / tree | `d11d99131d63e5fded7de6e0a7f6c7fb8f473913` / `280feb50066965fa1eb995ecd77ad53baf7b0244` |
| Prior reviewed candidate | `0b6e94db7faa4bee3d2ae88a5d2da2a5567e05be` |
| Accepted specification / report continuation | `adfb44a2aab2e29ba6574c9dacc09475c534bccf` / `d1522594fc16ea884c436e73fe08b0fdabc4a14f` |
| Candidate source TAR SHA-256 | `d2c647058e4e8073939086ea0558aeafabb88a0e5fc04ad90cf003ff36d0a9af` |
| Public-base → candidate diff SHA-256 | `fe579ad9ab34e624f1b42de53c825ebc2539555f5a21db9dc9d1f6b1e6ee33d4` |
| Cargo.lock SHA-256 | `97bc0a2b02ae77e013d45a47064b61b40dd6c6c6f495be5d245391cee8083cf8` |
| rust-toolchain.toml SHA-256 | `f715f440167e20fd8ea9afa0498c3d1ab80174cfcb8cbc895f106a34d907a1e1` |

Tracked input was clean. Immutable Git objects define this candidate; other reviewers' distinct untracked outputs are excluded. Source artifact reproduction: `git archive --format=tar 54cde8e317ed0ea98d88255b9f12cb9a1151229b | sha256sum`. Diff reproduction: `git diff --no-ext-diff --no-renames --full-index --binary BASE CANDIDATE`. [Static review evidence](evidence/bootstrap-quality-r2/static-review.json) records full interval hashes/path lists, source continuity, selected input hashes and independently calculated values.

Declared toolchain: Rust/Cargo **1.99.1**, edition 2024, minimal plus rustfmt/Clippy. Execution evidence identifies rustc **58cf8f9c35b355199727627d07ae34a4e2846d67**. Lockfile: three local packages, zero third-party Rust dependencies. The author's debug diagnostic SHA-256 **cb8e2b261e573bf38904e605775160eedc4e860959dfbc41bbd26e8be9b101b4** belongs to the source-changing parent; it is not a release build or measured native artifact. No benchmark artifact, approved frozen native workload, actual-profile trace or qualified collector result exists.

## Change and evidence binding

The complete intervening changes were checked by path, relevant source/document content and hashes. **0b6e94d → 68e6f52** adds round-1 reports/evidence and changes only README/SECURITY/IST among existing files (44 paths). **68e6f52 → d11d991** changes 31 paths for identity/UI/guard/tests, media/allocation decisions and associated documentation/evidence. **d11d991 → 54cde8e** changes only the remediation report and IST and adds three committed-source evidence files. All executable/contract inputs are identical across that final continuation.

Every one of the author's **216** source-manifest paths, content hashes and executable flags matches committed d11d991; all ten recorded command before/after source digests agree. Source-parent TAR SHA-256 is **2ce839ff80315aa7adcbf8266cb799fec1f4b11bb8a3ee78f919f5fcb581eebd**. This binds the author results; it does not turn them into reviewer execution. No pre-existing public-base review/evidence path changed. Accepted specification paths retain their bytes except the previously authorized additive revision-05 anchor; budgets and workloads were not amended.

| Requirement / assertion | Independent assessment | Evidence and limits |
| --- | --- | --- |
| Q.PERF-05 native backing bounds | **Pass for finite construction/admission design; BPR1-01 closed below.** | Media allocation §§A–E and ADR-005; independent arithmetic. No allocator/hook enforcement or native measurement. |
| Q.PERF-01/02/07/08 workload and complete-lifetime accounting | **Pass for unchanged contracts and additive media accounting.** | Original measurement text is an identical prefix; six appended lines bind native ledger/generations/helpers without exemptions. No qualified collector or run. |
| Q.PERF-04 policy costs and timing | **Pass for retained accounting/arithmetic design.** | ADR-004 refresh-cost section unchanged; no measured CPU, traffic or wakeups. |
| Q.PERF-05 non-media/UI/identity bounds | **Pass for reviewed scaffold/design scope.** | Data-only exports, explicit lexical limits, bounded supervisor slots; actual actors/queues remain absent. |
| Exact final CI | **Pass for recorded bootstrap checks.** | [Run 39150907632](https://github.com/adapt2move/nextcloud-modern-talk/actions/runs/39150907632), job 117744177812: docs 64/779/0 errors, formatting, locked compile, Clippy, **16 tests**. [Independent CI inspection](evidence/bootstrap-quality-r2/ci-inspection.json). |
| Extra diagnostic/probe/boundary checks | Attributed author passes, source-bound and inspected. | [Committed checks](evidence/bootstrap-remediation-r1/committed-checks.json): diagnostic, 36 probe assertions and 22 boundary cases; **not executed by CI or this reviewer**. Original format/lint failures remain preserved. |
| Native/reference-hardware performance | **Not run / unverified.** | No release client, actual media, collector qualification, workload validation, p95/peak, CPU, power, startup or latency evidence. |

Reviewer methods were static Git reads, Python standard-library hashes/JSON/integer arithmetic and primary-source HTTPS retrieval, using Git 2.47.3/Python 3.13.5 on Linux x86_64. No project code/checker/fixture was executed or imported; no project tests ran on the host and no VM lifecycle action occurred. Arbitrary-precision arithmetic audits the specification only; it does not exercise production fixed-width overflow behavior.

## BPR1-01 — Medium: verified fixed for finite native allocation decisions

**Original finding:** [performance round 1](bootstrap-performance-review-r1.md) identified that ADR-002's media row and ADR-004 bounded queued frame counts/audio duration but left backing dimensions/formats, native capture/decoder references, pools, in-use/shared/GPU storage and aggregate audio bytes undecided. Reproducer: a nominally compliant one-screen/two-frame queue could hold a 16,384×16,384 four-byte surface, **1 GiB before padding/references**. That was an arithmetic counterexample, not an observed stream. Expected: a finite profile/allocation rule applied before oversized native allocations. Actual then: no such rule. Impact: a missing consequential Q.PERF-05 bootstrap resource decision, not a measured budget failure.

**Owner and author response:** media/platform and architecture contributors; corrected by **d11d99131d63e5fded7de6e0a7f6c7fb8f473913**, unchanged in the final candidate. Rechecked [media-allocation.md](../architecture/media-allocation.md), lines 5–109, [ADR-005](../architecture/decisions/005-native-media.md), lines 23–91, and the reconciled resource table/measurement contract. Acceptance required independently derivable controlled maxima, pre-allocation rejection, explicit ownership and generation/concurrency accounting; no native benchmark was required at G.BOOT.

### Admitted shape, layouts and byte arithmetic

The chosen initial profile is VP8 8-bit 4:2:0 and Opus. Camera dimensions are positive/even and at most 1280×720, baseline 30 fps; screen at most 1920×1080, baseline 10 fps. One encoding/rendition per source is allowed. CPU/native layouts are I420/NV12 or BGRA8 for specified capture/conversion/presentation; arbitrary HDR/high-bit-depth/chroma/texture/mipmap/multisample formats are not implicitly admitted. Local maximum is one microphone/camera/screen; remote maximum is four cameras, one screen and eight audio publishers, reduced by stricter policy. Preview aliases the same local camera pipeline.

Checked arithmetic covers every addition, multiplication, alignment, plane span/offset and native signed conversion. Chosen YUV padding is at most 64 pixels per edge, with 64-byte stride alignment and 16 KiB per-plane accounting. These are admissible envelopes, not assertions about native allocator/page-size behavior. Greater native backing/strides, opaque/tiled allocations without a qualified bound, unsupported planes or larger input dimensions are rejected/blocked before allocation; post-allocation downscaling cannot repair admission.

I independently derived the maximal layouts using `A(x,n)=ceil(x/n)*n`, not by trusting the author's totals:

| Frame class | Independent calculation, bytes | Reserved slot |
| --- | ---: | ---: |
| Camera I420 | 1,196,032 luma + 2×311,296 chroma = **1,818,624** | 2 MiB |
| Screen I420 | 2,473,984 luma + 2×622,592 chroma = **3,719,168** | 4 MiB |
| Camera NV12 | 1,196,032 + 606,208 = **1,802,240** | Within the I420 reservation |
| Screen NV12 | 2,473,984 + 1,245,184 = **3,719,168** | Within the I420 reservation |
| Camera BGRA | **3,686,400** | 4 MiB |
| Screen BGRA | **8,306,688** | 8 MiB |

Each aligned-plane expression is nondecreasing over admitted positive/even dimensions, so these endpoints bound that layout domain. The approved NV12 chroma stride is no greater than the sum of the two approved I420 chroma strides; separate plane rounding preserves the stated upper bound. Actual allocation size, padding and auxiliaries must still fit the selected charge. The old 16384-square counterexample now violates profile admission before any permitted allocation; **that response is specified, not implemented**.

### Pool assignments, compressed storage, audio and auxiliary state

Each logical camera/screen stream reserves **12 YUV slots and 8 BGRA slots**. The partition is important:

- YUV: eight slots **collectively** for native capture/encoder/decoder/reference storage, two waiting handoffs, one conversion/processing input and one renderer-held source. It is not eight slots for each native component.
- BGRA: three native capture-source buffers, one conversion/upload scratch, one current presentation, two GPU in-flight uses and one additional retained presentation/handoff backing. Aliases share a charge; separate storage/copies consume separate slots. Either format still shares the two-waiting-application-frame limit.

Thus camera surfaces are **12×2 + 8×4 = 56 MiB**, and screen surfaces **12×4 + 8×8 = 112 MiB**. The simultaneous capture/conversion/presentation allowance does not depend on assuming zero-copy. Decoder references, backing that never reaches a callback, old format generations and independent render/readback copies must fit those same pools; there is no private overflow pool.

The three-entry ScreenCaptureKit source queue is separately identified from the two waiting application frames and included in the three BGRA capture slots. I retrieved Apple's [queueDepth documentation](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/queuedepth): its minimum/default is three, making the reconciliation meaningful. The [Core Video threshold result](https://developer.apple.com/documentation/corevideo/kcvreturnwouldexceedallocationthreshold) concerns allocation through the specified pool API; it does not cap other framework/codec/GPU allocations. Both points remain native qualification obligations, not guarantees derived from an API name.

Compressed video adds **8 MiB per logical video stream**: eight 1 MiB units divided into two each for reassembly, transform input/output, working bitstreams and pacing/retransmission/packets. Complete encrypted frames are at most 1 MiB; up to 64 bytes of transform expansion must be reserved before plaintext admission, and any eventual transform needs a proved bound. Fragment/packet metadata and every copy stay in the arenas. Ingress must be bounded before RTP assembly/native growth; encoder output must also be reserved before creation. Unknown expansion or interception coverage blocks the path.

Audio adds **4 MiB per call**. Twelve PCM lanes cover local input, eight remote streams, mixed output, AEC reverse/reference and conversion. Worst admitted 100 ms float32/stereo PCM is **48,000×2×4×0.1 = 38,400 bytes** per lane. Two 64 KiB blocks per lane total **1,572,864 bytes (1.5 MiB)** within the 2 MiB PCM pool. Another 1 MiB is encoded/packet storage and another 1 MiB sample/history/processing storage. Nine publish/receive streams × five 4 KiB packets = **184,320 bytes (180 KiB)** before extra copies, which share the 1 MiB arena. The 100 ms waiting duration is aggregate across encoded/jitter/PCM stages, not an extra allowance at each stage. Native histories/driver buffering cannot be assumed zero or hidden outside the caps.

Control adds **8 MiB per call**, including native parsing/copies, bounded callback/IPC descriptors, key records and data-channel queues. Supplemental object bounds are 12 PeerConnections, 24 transceivers, two data channels per connection, 64 KiB complete SDP and at most 64 candidates per side/connection at 2 KiB each. Object limits do not supersede the byte cap; critical revoke flags bypass ordinary queues.

The separate **128 MiB/helper plus 8 MiB/watchdog auxiliary ceiling** covers private metadata, bounded algorithm tables/tasks and owned thread stacks. It is expressly unproved and is neither a libwebrtc setting nor permission to move video references, PCM/history, bitstreams or packets out of their explicit pools. Every helper allocation needs a reservation path or finite audited preallocation bound before enablement. Binary/runtime mappings, fragmentation, kernel/driver/GPU overhead and system services remain physical-footprint/attribution costs; proportional media backing cannot be excluded under those labels.

### Aggregate accounting and concurrency

For C logical camera streams and S screen streams, controlled buffers are **64C + 120S + 12 MiB**. I independently recomputed each profile:

| Profile | C / S | Surfaces | Compressed | Audio + control | Controlled total |
| --- | --- | ---: | ---: | ---: | ---: |
| Explicit camera preview | 1 / 0 | 56 | 8 | 12 | **76 MiB** |
| Direct audio | 0 / 0 | 0 | 0 | 12 | **12 MiB** |
| Direct video, local + remote camera | 2 / 0 | 112 | 16 | 12 | **140 MiB** |
| Four-person HPB conference, local + three remote cameras | 4 / 0 | 224 | 32 | 12 | **268 MiB** |
| Same conference, local screen sender | 4 / 1 | 336 | 40 | 12 | **388 MiB** |
| Same conference, remote screen receiver | 4 / 1 | 336 | 40 | 12 | **388 MiB** |
| Maximal slot, local + four remote cameras and local + remote screen | 5 / 2 | 504 | 56 | 12 | **572 MiB** |
| Two maximal slots | 10 / 4 | 1,008 | 112 | 24 | **1,144 MiB** |

The Rust media supervisor owns one **application-wide** ledger and call subledgers spanning helpers and renderer imports. At most one slot/helper per account and two globally includes previews and unreclaimed generations. Preview-to-call reuses its slot. An old helper or renderer-held mappings cannot be bypassed by creating an uncharged replacement. Native references, callback quiescence and GPU fences determine credit return; process death, dropped queue entries or invisible surfaces do not. If retained storage prevents admission, the new operation is Busy/blocked. Other accounts' quota is not silently borrowed.

Including the separate auxiliary ceilings gives **2×(572+128)+8 = 1,408 MiB**, conditional on missing native allocation proof. The BGRA subset is **32C + 64S MiB**, 288 per maximal slot / 576 for two; YUV may also be GPU-visible, so this is not total GPU memory. Aliases count once in the allocation ledger while independent GPU copies count separately. Measurement must report raw counters/overlap without summing overlapping unified-memory values. The 32 MiB decoded-image and 64 MiB GPU-image caches are separate, not substitutes for these media reservations.

These are **lazy reservation maxima, not eager allocations, RSS/physical-footprint results or enlarged performance budgets**. Preview/maximal/two-slot scenarios have no inherited accepted budget pass. The screen sender and receiver retain all four baseline cameras and remain different workloads despite equal arithmetic. Original direct-video 450/600 MiB, conference 700/900 MiB and screen 850/1,100 MiB p95/peak targets remain unchanged, with the main application/helpers and non-buffer costs still measured.

### Native enforcement feasibility and disposition

The contract does not pretend that a wrapper callback already intercepts every native allocation. Independent reads of the immutable sources confirm concrete limitations:

- [VideoDecoder settings](https://webrtc.googlesource.com/src/+/a306485b7fc0fe38dd2651420e383f92f196cd77/api/video_codecs/video_decoder.h) impose a caller resolution obligation and permit pool-size settings to be ignored. The [VP8 wrapper](https://webrtc.googlesource.com/src/+/a306485b7fc0fe38dd2651420e383f92f196cd77/modules/video_coding/codecs/vp8/libvpx_vp8_decoder.cc) invokes libvpx decode before creating/copying its output-pool frame; that pool cannot establish the earlier reference/scratch bound.
- The pinned [VP8 decoder](https://chromium.googlesource.com/webm/libvpx/+/5e680f30801d03c21078f8c4b772464752516211/vp8/vp8_dx_iface.c) has a pre-initialization keyframe dimension check under CONFIG_SIZE_LIMIT. Chromium's pinned [generator](https://chromium.googlesource.com/chromium/src/third_party/+/8a83e63525b495aa7f35648a3b4b676279deb864/libvpx/generate_gni.sh), [arm64 configuration](https://chromium.googlesource.com/chromium/src/third_party/+/8a83e63525b495aa7f35648a3b4b676279deb864/libvpx/source/config/linux/arm64-highbd/vpx_config.h) and [BUILD.gn](https://chromium.googlesource.com/chromium/src/third_party/+/8a83e63525b495aa7f35648a3b4b676279deb864/libvpx/BUILD.gn) confirm the selected macOS arm64 path's **16384×16384** guard. A reviewed/generated **1920×1080** limit plus the stricter camera check is still required; the size guard alone does not bound allocation counts or all scratch storage.
- The [external frame-buffer API](https://chromium.googlesource.com/webm/libvpx/+/5e680f30801d03c21078f8c4b772464752516211/vpx/vpx_decoder.h) is VP9-only. [EncodedImageBuffer](https://webrtc.googlesource.com/src/+/a306485b7fc0fe38dd2651420e383f92f196cd77/api/video/encoded_image.cc) allocates/resizes from supplied sizes. Those facts preclude claiming a ready universal VP8 preallocation hook.

[Primary-source evidence](evidence/bootstrap-quality-r2/primary-sources.json) records independent retrieval/hashes and reader fallbacks. **Unmodified upstream and every native media profile remain unqualified/blocked.** Enabling one needs a complete allocation-path inventory, wrappers/configuration/source patches or proved fixed bounds, including RTP, encryption expansion, capture-source pools and GPUI imports before allocation. Killing a helper after excess, sampling footprint or setting an output-pool parameter cannot replace that proof.

**Disposition: verified fixed for BPR1-01's missing finite resource decision.** The numbers, ownership, overload/reclamation and before-allocation rules are now concrete enough to evaluate and implement. No new size primitive was introduced that requires a production arithmetic test at this gate. Native denial/overflow/lifetime tests and allocation feasibility remain with media/platform and independent verification before profile admission; actual performance still needs qualified workloads/collectors. This is contract closure, not native containment verification or an exception for unknown allocations.

## Frozen workloads, peak collection and other regression checks

**QR1-04/05 remain closed as accepted specification findings.** All quality/workload-template bytes are unchanged from the accepted specification. The original **62 lines of measurement.md are an identical prefix** of this candidate; the only change is six appended lines specifying media-ledger observations, helper/watchdog lifetimes, retained mappings and the distinction from measured budgets. No peak, workload or quality tolerance was relaxed.

The retained workload contract assigns strict versioned manifest parsing, deterministic fixtures/actions and prepare/replay/validate/summarize to the lab owner; application supplies action/residency observations, media supplies negotiated **and delivered** per-stream series, and performance review freezes the inputs before runs. Two independent reconstructions must agree. It preserves active-chat counts, large-history page/cache/search mix, camera-plus-screen directions, quality/activity windows and missing-observation handling. Dropped actions, warm-cache substitution, lowered layers/quality or a missing camera invalidate the original workload. Safe runtime degradation is not a performance pass. No approved executable acceptance manifest or actual-profile validation is supplied.

The retained collector observes prelaunch through final exit, including startup/teardown and short-lived/nested/reparented/crashed/killed helpers, birth identity/PID reuse and gaps. One-second aggregate footprint p95/sampled maximum remain separate from lifetime high-water. `U = sum(all owned lifetime maxima)` is a conservative upper bound, not a simultaneous measured peak; an excessive loose U needs a qualified tighter bound, and missing evidence is incomplete. GPU/shared/system-service overlap, all owned CPU, control/TLS/retries, package power and collector overhead stay included. The new watchdog and retained renderer mappings receive no exemption.

The plan retains the 100 ms touched 64 MiB burst, 300 ms helper, between-tick ceiling breach, abnormal exits, PID reuse and missing-observation qualification cases. Earlier pinned Apple-header research is carried with its original attribution through unchanged source-mapping/collector text; it was not re-fetched for a new native API claim. Actual access, abnormal-exit high-water recovery and complete GPU coverage on M2/macOS 15.7 remain unresolved. A faster sampler or final voluntary report is still insufficient.

**Policy costs:** ADR-004's source/freshness/profile sections before local enforcement and its refresh-cost section are byte-identical to round 1. Independent arithmetic confirms S+R+D = 2+10+3 = 15 s, request-start expiry offset 13 s, at most 35 starts/minute/call, 322 KiB/min estimate ≈43.96 kbit/s combined and 512 KiB/min envelope ≈69.91 kbit/s. These consume existing active-call budgets; directional TLS/signaling/retries and bounded in-flight overrun remain counted. One timer wake/s and 0.5% of a logical CPU per call are unmeasured planning allowances. Secure refresh cannot be relaxed to fit a budget, and active-call polling is not hidden in signed-in idle.

**Shutdown reconciliation:** 200 ms unacknowledged-publication escalation leaves only 50 ms to qualify termination within the unchanged 250 ms publication deadline. The 750 ms escalation and 1,000 ms independently observed device-release deadline remain. A stalled renderer cannot release allocation credit or keep devices open under a falsely reported stop. There is no OS scheduling/kill guarantee; missed deadlines fail native qualification, and unobservable release is incomplete. These new ownership details do not enlarge the accepted bounds.

**Non-media resources:** prior bounded snapshots/pages/history/requests/transfers/caches and worker ownership remain; the UI change exports reviewed bounded data and IDs rather than adding a runtime collection or media path. The strengthened direct-container guard and grouped import rules retain explicit lexical/type-resolution limits. Identity maintenance adds finite eight-descriptor/account, one-provider/account, two-worker-global bounds and a 3 s observation slice; uncertain writers retain slots instead of causing replacement-worker growth. Those actors/queues remain unimplemented. No performance regression or budget feasibility can be inferred from the diagnostic's tiny workload.

## Prior dispositions, carried evidence and handoff

| Finding/history | Disposition and owner |
| --- | --- |
| **BPR1-01, medium** | **Verified fixed for finite native resource contracts**, above. Media/platform owns native enforcement/coverage and later performance qualification. |
| **BSR1-01 / BAR1-01, high, one defect** | Separate [security round 2](bootstrap-security-review-r2.md) closes its security contract objection; architecture/function retain their separately attributable conclusions. No performance objection to the bounded supervisor correction; no durable/runtime proof inferred. |
| **BSR1-02, medium** | Separate security report closes the required engine/binding decision. Selected libwebrtc/DEPS/IPC ownership is not licensing, interoperability or performance clearance. |
| **BAR1-02, medium** | [Architecture](bootstrap-architecture-review-r2.md) and [function](bootstrap-functional-review-r2.md) round 2 separately record verified-fixed UI-boundary closure. Static data-only surface inspected here; no renderer performance pass. |
| **BFR1-01, medium** | Original exact BTreeMap negative retained; new guard and intended author diagnostic inspected. Functional round 2 separately records closure. General parser/transitive-bound guarantees are not claimed. |
| **BI-001** | **Verified fixed, carried forward:** anchor/reference/checker bytes unchanged since round 1; original negative evidence retained and final CI docs scan passes. No fixture rerun here. |
| **QR1-01–05, PR1-01–03, DR-001–004** | Accepted specification/design dispositions retain byte continuity and original scope, including workload/peak and policy-cost requirements. No native performance or repeated design audit follows. |

All five backend harness/support hashes match the original binding. The independent round-1 report SHA-256 remains **6874334d2fec5fc56ad1bf82ef48496746277c43c00d538bc1263dc26263603f**, with NC34.0.5/Talk24.0.6 **96 pass + 1 N/A** and NC35.0.2/Talk25.0.6 **111 pass**. This is attributed API evidence, not a native workload, app-password/media test or complete CT pass. The contracts-only changes do not justify a backend rerun. Unchanged CI/lab/security controls are carried through inspected diffs; live publication/CI settings are separately recorded, with no performance implication.

I reached this performance conclusion before consulting the concurrent architecture/functional round-2 dispositions; their consulted hashes are separate from candidate hashes. All original objections are retained. **New BPR2 findings: none.** No further native or project run is required to decide this contract-only correction; the absent native allocation/workload/collector proof remains mandatory before its product claims.

IST's fixed-pending/unaccepted state describes the reviewed input. PM/integrator should integrate the new separately attributable conclusions in a later authorized continuation. I made no source, specification, IST, feature-matrix, checker, old-report, repository configuration, commit or push change. Only the assigned two new reports and static evidence were written.

The parent VM's snapshotting/image-quota block is a supplied operational fact, not a fresh observation here. No lifecycle operation, alternate VM, host project execution or successful new hibernation is claimed. Historical snapshot evidence is not current-state proof.

**Final confirmation:** performance contracts/scaffolding pass for **54cde8e317ed0ea98d88255b9f12cb9a1151229b**, 2026-10-18. Product security, media integration and native performance remain unverified. Root PM retains gate acceptance.
