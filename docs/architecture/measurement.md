# Measurement interfaces and qualification plan

Status: bootstrap contract, **no workload runner, validator, collector or benchmark is implemented here**. Q.PERF-01/02/07/08 and V.PERF.PEAK/WORKLOAD remain authoritative; this assigns owners and concrete integration seams without claiming an approved workload.

## Ownership and executable workload seam

The lab/testing contributor owns versioned manifest files, synthetic generators, deterministic trace replay and actual-versus-planned validation under `tests/lab/` and `docs/testing/`. The application owner supplies typed intent injection and nonsecret action/result/residency counters; the media owner supplies per-stream negotiated **and delivered** observations; the platform owner supplies the collector adapter below. The independent performance reviewer freezes each workload and qualification manifest before acceptance runs. Two independent runners reconstruct and compare the normalized plan. The PM accepts resulting evidence, not the author or tool exit code alone.

The [existing manifest template](../quality/workload-manifest-template.md) is the human review source. The future executable representation is UTF-8 JSON, schema `workload-v1`, rejecting unknown keys, missing values, duplicate object keys, nonfinite numbers and unresolved placeholders. It has these required top-level objects; durations/timestamps are integer nanoseconds relative to the declared monotonic epoch, sizes are bytes, hashes are lowercase SHA-256 hex. A schema/fixture change creates a new reviewed profile; no tolerance is changed after observing a result.

| Object | Required fields and validation |
| --- | --- |
| `identity` | schema version, workload/profile ID, reviewed manifest revision, generator/tool/artifact hashes, reviewer evidence reference; hash exact manifest bytes outside the file to avoid a self-hash cycle |
| `environment` | exact client/toolchain/lock/signing/OS/SDK/hardware/display profile, server/Talk/HPB/TURN artifacts/topology and capability/config fixture hashes; no secrets or runtime private paths |
| `phases` | prelaunch collection, launch, warm-up, steady, teardown, final-exit boundaries, repetitions, reset/cache policy; every interval finite and ordered |
| `fixtures` | generator revision/seed, corpus/assets hashes and exact lengths/distributions/overlap, account/room/message mapping; deterministic synthetic source data only |
| `actions` | ordered records with stable sequence ID, due time, typed action/target/argument hash and expected outcome/count; exact paging/search/scroll/typing/switch/send schedules, residency/eviction and cancellation points |
| `streams` | one record per sender→receiver stream: codec/layer/processing path, audio rate/channels/activity, camera/screen dimensions/fps/bitrate, deterministic source asset, tile/preview visibility and subscription schedule; sender/receiver screen profiles separate |
| `tolerances` | per-action timing/count limits, paging/residency conditions, per-stream quality/delivery windows and maximum observation gaps; zero count deviation where the workload fixes exact counts |
| `collection` | collector/qualification IDs and hashes, sampling cadence, lifetime/peak method and error bounds, owned/system-service attribution, CPU/network/GPU/power boundaries and overhead procedure |

Define four executable operations for the lab-owned driver; their implementation/CLI name is pending, **these are not runnable commands today**:

1. `prepare(manifest, fixture_store) -> normalized_plan + hashes`: parse strictly, verify source hashes, reproduce fixtures and expand all cadence descriptions into exact actions/stream schedules. No random draws depend on wall time or execution speed. An unapproved/missing field returns `InvalidManifest`.
2. `replay(plan, typed_application_port, stream_driver, observer) -> actual_trace`: run exactly the frozen schedule, including failed/busy/cancelled actions; do not silently omit actions. Backpressure is an observed failure or permitted explicit outcome, not a change to counts. Deterministic media must move/speak; static frames or muted silence do not substitute.
3. `validate(plan, actual_trace, stream_series, lifecycle_evidence) -> verdict + deviations`: compare sequence IDs, target identities, counts/times, results, cache/page residency, negotiated/delivered quality, frame/audio activity and observation gaps against predeclared tolerances. Missing observations are incomplete; lighter load/quality is invalid, never a budget pass.
4. `summarize(validated_run, collection) -> acceptance_evidence`: only a valid workload with complete qualified collection can reach budget evaluation. Preserve raw traces and worst runs. Two reconstructions must produce identical normalized-plan hashes before native acceptance measurements.

Each observation record has schema, run ID, sequence, clock epoch/time, source owner and kind. Allowed kinds are action-start/result, cache-page-state, stream-negotiated, stream-delivered, policy-source/expiry/enforcement, queue-depth and phase-change. Stream-delivered includes stream ID, sample interval, frame/audio counts, dimensions/rate/layer/processing path and gaps. Use bounded records (≤64 KiB); collector backpressure/record loss invalidates completeness rather than silently dropping evidence. Never include message text, RTP/frame bytes, credentials, endpoints with tokens or private file paths.

## Complete process and peak collector

The platform performance owner provides `CollectorSession` with this lifecycle:

| Operation | Required contract |
| --- | --- |
| `arm(profile, qualification) -> session` | Start observation before process launch; freeze clock/metric definitions, permissions, API revisions and overhead counters. Fail if required native collection is unavailable. |
| `register_spawn(owner, executable_hash, birth_ticket)` | Account for each app/helper/descendant before it runs. Match with independent OS birth observation; parent PID alone is insufficient. System-mediated services are separately attributed. |
| `observe_lifetime(event)` | Track birth identity `(boot/session ID, PID, OS birth/start identity)`, parent/ownership lineage, spawn/exec/reparent/exit, signing/executable identity and gaps through final exit. PID reuse creates a new record. |
| `sample(moment)` | Once per second: every live owned process's physical footprint, RSS (secondary), CPU cumulative time and collector provenance; aggregate footprint from simultaneous-enough samples with timing error recorded. Missing processes are not zero. |
| `close_lifetime(birth_id, evidence)` | Retain physical-footprint **lifetime high-water**, source/API/structure revision, units, time coverage, exit mode and uncertainty. Evidence must survive crash/forced termination; voluntary final reports alone do not qualify. |
| `finish_after_final_exit()` | Prove no owned lifetimes remain/open or were missed; emit aggregate p95/sample maximum, per-process maxima, `U`, qualification/error bounds and `Complete`/`Incomplete`. Unknown/lost peak evidence yields incomplete. |

Candidate macOS live metrics use `TASK_VM_INFO.phys_footprint`; potential lifetime sources include `ledger_phys_footprint_peak` and `rusage_info_v4.ri_lifetime_max_phys_footprint`, declared in the [pinned Apple headers](source-evidence.md#a-src-05--native-measurement). This is a research candidate, **not a claim of retrieval rights, structure compatibility or exited-process coverage on macOS 15.7**. The implementation must verify returned structure size/revision and kernel semantics on the exact SDK/OS. It must never relabel `resident_size_peak`, RSS `ru_maxrss`, allocator bytes or a sampled footprint maximum as physical-footprint lifetime high-water.

The planned design combines runtime-owned spawn registration with an external OS lifetime observer and native live/high-water queries. A future implementation must demonstrate a retained exit/termination evidence path for normal, crashed, killed, short-lived and reparented helpers. No documented universally usable exited-task API has been established by this bootstrap. If the selected OS path cannot recover an abnormal exit's high-water, mark the run incomplete; a faster sampler or last volunteered report cannot repair that gap. This is a collector qualification blocker for native memory acceptance, not an excuse to report zero.

With complete lifetime evidence, compute `U = sum(high_water_bytes for every owned lifetime)`. `U` is a conservative bound on simultaneous aggregate footprint; it is **not** a measured simultaneous peak. Apply workload peak ceilings across launch, warm-up, steady and teardown. If `U` is too loose/large, a separately reviewed concurrent bound with explicit temporal/size error may replace it. An observed ceiling breach fails; missing evidence is incomplete. Keep p95 and sampled maximum as distinct metrics.

GPU resources/shared surfaces and WindowServer/system-media deltas need separate native collectors, stated overlap and missing-cost fields. Do not sum overlapping unified-memory metrics or claim total client memory while omitting costs. CPU includes all owned helpers and 100% means one logical core. Network includes control/TLS/retries/media; power is measured package delta, not battery life. Capture driver/helper cost belongs to the native run even when the app uses an isolated media process.

## Qualification procedure

The lab and platform owners author a frozen `collector-qualification-v1` manifest before performance review. It fixes fixture hashes, touched byte sizes, burst/phase times, process topology, termination mode, CPU/clock overhead comparison and expected outcomes. Qualification must include:

- A **100 ms** touched-memory burst of **64 MiB** wholly between the ordinary 1 s samples, with independently observed actual touched pages. Also inject a known workload-ceiling breach; the peak safeguard must catch it even if every regular sample misses it.
- A child whose entire **300 ms** lifetime is between samples, plus nested and reparented variants; allocate in startup and final teardown as well as steady state. Every lifetime must have a distinct birth identity and peak evidence.
- Normal exit, crash and forced kill during an allocation; rapid PID reuse fixture/controlled trace replay plus a real OS lifetime test. No optional self-report may be the only source for abnormal-exit evidence.
- Clock/sleep/observer gaps, access denial, unsupported structure revisions, collector backpressure, missing final peak and deliberately omitted child. Each must prevent a complete memory pass.
- Measured collector CPU/memory/timing/network overhead and independent allocation observation; retain required accounting instrumentation in headline runs. Heavier profiling is separate. Fixture sizes/durations above are selected test inputs, not already executed or approved measurements.

For workload qualification, two runners reproduce identical fixture/plan hashes; intentionally change active-chat message count, search/residency pattern, camera quality, camera-plus-screen directions and screen role. The validator must reject each incompatible workload rather than accept a smaller resource result. Run all V.PERF.PEAK/WORKLOAD cases and retain exact manifests, commands, binaries, raw time series, normalized deviations and independent review. Linux server/API tests cannot pass these macOS native gates.
