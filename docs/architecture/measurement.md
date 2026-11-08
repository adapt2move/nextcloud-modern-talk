# Performance measurement contract

Use the [performance requirements](../quality/performance.md) for workload budgets
and [verification cases](../quality/verification.md#vperfpeak--complete-process-and-peak-evidence)
for completeness checks. This defines acceptance measurements, not a shipped
collector API or a claim that a budget has passed. The bounded diagnostic runners
in [development](../development.md) establish only their stated Linux scope.

## Ownership and executable workload seam

The lab/testing contributor owns versioned manifest files, synthetic generators, deterministic trace replay and actual-versus-planned validation under `tests/lab/`, with run outputs in private evidence outside contributor documentation. The application owner supplies typed intent injection and nonsecret action/result/residency counters; the media owner supplies per-stream negotiated **and delivered** observations; the platform owner supplies the collector adapter below. The independent performance reviewer freezes each workload and qualification manifest before acceptance runs. Two independent runners reconstruct and compare the normalized plan. The PM accepts resulting evidence, not the author or tool exit code alone.

An acceptance workload manifest uses UTF-8 JSON, a versioned schema, rejecting unknown keys, missing values, duplicate object keys, nonfinite numbers and unresolved placeholders. It has these required top-level objects; durations/timestamps are integer nanoseconds relative to the declared monotonic epoch, sizes are bytes, hashes are lowercase SHA-256 hex. A schema/fixture change creates a new reviewed profile; no tolerance is changed after observing a result.

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

Native metrics need validation on the exact SDK/OS, including permissions, API
revisions and abnormal exits. RSS, allocator bytes and sampled maxima are not
physical-footprint lifetime peaks. Missing exit data makes collection incomplete.
With complete lifetime data, the sum of all owned lifetime high-water values is a
conservative aggregate bound, not a measured simultaneous peak. A looser bound above
the budget needs a qualified tighter collector; a sampled maximum cannot repair it.
Report GPU/shared-surface and system-service costs separately with overlaps stated.
CPU includes helpers; network includes control, retries and media; power is measured
package delta, not battery life.

## Qualification procedure

The lab and platform owners author a frozen `collector-qualification-v1` manifest before performance review. It fixes fixture hashes, touched byte sizes, burst/phase times, process topology, termination mode, CPU/clock overhead comparison and expected outcomes. Qualification must include:

- A **100 ms** touched-memory burst of **64 MiB** wholly between the ordinary 1 s samples, with independently observed actual touched pages. Also inject a known workload-ceiling breach; the peak safeguard must catch it even if every regular sample misses it.
- A child whose entire **300 ms** lifetime is between samples, plus nested and reparented variants; allocate in startup and final teardown as well as steady state. Every lifetime must have a distinct birth identity and peak evidence.
- Normal exit, crash and forced kill during an allocation; rapid PID reuse fixture/controlled trace replay plus a real OS lifetime test. No optional self-report may be the only source for abnormal-exit evidence.
- Clock/sleep/observer gaps, access denial, unsupported structure revisions, collector backpressure, missing final peak and deliberately omitted child. Each must prevent a complete memory pass.
- Measured collector CPU/memory/timing/network overhead and independent allocation observation; retain required accounting instrumentation in headline runs. Heavier profiling is separate. Fixture sizes/durations above are selected test inputs, not already executed or approved measurements.

For workload qualification, two runners reproduce identical fixture/plan hashes; intentionally change active-chat message count, search/residency pattern, camera quality, camera-plus-screen directions and screen role. The validator must reject each incompatible workload rather than accept a smaller resource result. Run all V.PERF.PEAK/WORKLOAD cases and retain exact manifests, commands, binaries, raw time series, normalized deviations and independent review. Linux server/API tests cannot pass these macOS native gates.

## Shipping media topology

The shipping Rust WebRTC engine runs in-process with GStreamer codecs. Measure
actual native allocations, device queues, GPU resources and platform helpers.
Application queue bounds do not cap decoder, OS or GPU allocations; there is no
implemented global reservation ledger or hard process-memory guarantee. Functional
interoperability and bounded diagnostic samples cannot establish full-profile budgets.
