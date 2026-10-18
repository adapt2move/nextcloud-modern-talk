# Performance requirements

Status: **provisional targets, not measurements**. No native-client benchmark has been run. A small bootstrap executable or HTML prototype cannot establish the product budgets below. Budget changes require an independent performance review and a recorded rationale, not retroactive reclassification of a failed result.

## Q.PERF-01 — Reference environment and accounting

Proposed reference: MacBook Air M2, 8 CPU cores, 10 GPU cores, 16 GiB unified memory; macOS 15.7, native arm64 release build. Record exact OS build, SDK, Rust/GPUI/media versions, compiler flags, signing, display resolution/scaling and hardware acceleration. Use a 1440 × 900 logical-pixel window at 60 Hz, effects disabled for the baseline. AC power, fixed 200-nit display, no Low Power Mode and no competing workload; record thermal state. Other Macs/OS versions get separate profiles. This proposal does not assert delivered OS support or available measurement hardware.

Use a pinned real-server profile for each supported pair in [the lab plan](test-servers.md). Record server host/resources/load, database, HPB/TURN topology and negotiated media. Baseline network: controlled 20 ms round-trip time, 100 Mbit/s in each direction, no injected loss; measure actual values. Run separate stress profiles at 150 ms RTT, 2% random packet loss and 2 Mbit/s per direction. Local UI latency is measured independently of server/network latency.

Steady memory metric: sum the macOS physical footprints (`TASK_VM_INFO.phys_footprint`, with collector/API revision recorded) of the app and every owned helper/child, sampled once per second. Include in-process media, decoder buffers and caches. Report the aggregate p95 and sampled maximum plus per-process footprint and RSS; RSS and allocator totals cannot substitute for footprint. One-second samples alone do **not** qualify the peak ceiling.

Track every owned process from before launch through its final exit, including startup, warm-up, short-lived helpers, descendants, reparented owned helpers and teardown. Identify a process by lifetime/birth identity as well as PID, so PID reuse cannot merge records. Capture spawn/exit times, ownership and a physical-footprint lifetime high-water value for each process, or a separately validated equivalent. A final voluntary helper report alone does not cover crash/forced termination; the collector must recover evidence for that path or mark the run incomplete. No unobserved process counts as zero. Report system-mediated services separately from owned processes.

The peak safeguard is separate from the sampled p95. Let `U` be the sum of every owned process's **lifetime** footprint high-water mark across the run. With complete qualified collection, `U` is a conservative upper bound on simultaneous aggregate footprint, not a measured simultaneous peak; sequential helpers can make it loose. `U` at or below the workload's peak ceiling satisfies this safeguard. If it exceeds the ceiling, qualify a tighter aggregate bound or concurrent peak measurement using a reviewed collector with explicit temporal/size error bounds; its upper bound must meet the ceiling. A sampled maximum below the ceiling cannot resolve an excessive or missing `U`. An actual observed exceedance fails; insufficient evidence is `incomplete`, never a memory pass. Do not sum RSS peaks or selected steady-state samples and label them `U`.

Apple's source declares `ledger_phys_footprint_peak` separately from `phys_footprint`. Its semantics, returned structure revision, access rights and collection for live/exited processes must be verified on the selected Mac/OS; presence in a header is not collector qualification. Qualify the chosen path under [Q.PERF-07](#qperf-07--evidence-and-profiling). [Apple task memory fields](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/task_info.h), [Apple memory profiling guidance](https://developer.apple.com/documentation/xcode/gathering-information-about-memory-use)

Attribute media/codec OS helpers where possible. Report GPU live-resource allocations, shared surfaces and WindowServer/system-media deltas separately, stating overlaps and unassignable memory; do not add overlapping unified-memory counters. A headline “total client memory” is forbidden when such costs are omitted or unmeasured.

CPU is summed process CPU time divided by wall time: 100% means one fully used logical CPU, not the whole machine. Include helpers. Energy is the measured CPU+GPU package power delta against the same machine's quiescent baseline, with collector and attribution limits recorded. It is not whole-device power or battery life. Measure per-app network bytes in each direction including signaling/media and retransmissions; state whether the collector includes transport headers. No silent switching of metrics between candidates.

## Q.PERF-02 — Workload budgets

All values below are proposed acceptance ceilings. Memory values are steady-state aggregate p95 / qualified peak ceiling, in MiB; CPU is interval mean. The peak ceiling also applies from process launch through final teardown, including warm-up; report these phases separately. There is no unreviewed startup/teardown exception. Report p95 CPU as evidence too. Use at least three independent process runs after five minutes of the manifest-defined warm-up, with the listed minimum steady duration per run; every run must meet both memory checks and the other ceilings.

The rows are workload summaries, not sufficient execution instructions. **Before acceptance measurements**, independent performance review must approve a complete, frozen workload manifest and its hash, using the [manifest template](workload-manifest-template.md). It fixes event cadence/counts, input trace, fixtures, cache/page conditions, media directions and quality, warm-up/reset and tolerances. A missing/unreviewed manifest or a result diverging from it is an invalid workload, not a budget pass. The bootstrap must assign the manifest owner and define its schema, replay/validation seams and collector contract; this phase supplies requirements, not an executed or approved workload.

| Workload | Fixed activity and duration | Footprint p95 / peak | Mean CPU | Package power delta |
| --- | --- | --- | --- | --- |
| Signed-in idle | One account, 100 conversations, synchronized, foreground static, 30 min | 160 / 200 | ≤1% | ≤0.5 W |
| Active chat | 500 conversations, 20 messages/min, typing and room switching, 10 min | 220 / 280 | ≤5% | ≤1.5 W |
| Large history | 100,000-message room; continuous paging, scrolling and search, 10 min | 300 / 380 | ≤15% | ≤3 W |
| Direct audio call | Two endpoints, bidirectional speech, camera off, 20 min | 250 / 320 | ≤25% | ≤2 W |
| Direct video call | Send/receive 720p30 plus audio, 20 min | 450 / 600 | ≤120% | ≤6 W |
| Four-person conference | HPB; local sends one 720p30 camera + audio and receives three remote 720p30 cameras + audio, 20 min | 700 / 900 | ≤220% | ≤8 W |
| Conference + screen, sender | Same camera/audio directions; local additionally sends one 1080p10 screen to the three peers, 20 min | 850 / 1,100 | ≤260% | ≤9 W |
| Conference + screen, receiver | Same camera/audio directions; one remote additionally sends a 1080p10 screen received locally, 20 min | 850 / 1,100 | ≤260% | ≤9 W |

Fixture messages average 400 UTF-8 bytes, include 10% longer messages up to 4 KiB, 5% attachment metadata and representative replies/reactions. Freeze the exact generator/revision/seed and corpus hashes, room/message distribution, payload-length definition and tolerances, avatar/thumbnail sizes and reply/reaction/attachment overlap. Active chat must total 200 messages over its 10-minute steady run; the manifest fixes incoming/outgoing split, burst/timing schedule, typing and room-switch events. Large history must fix page size, initial loaded/resident pages, scroll positions/velocities, eviction state and exact search query sequence/mix (hits, misses, broad/narrow, cached/uncached and cancellation timing). “Continuous” without rates/counts and residency limits cannot identify this workload.

Media uses hashed deterministic moving video, screen content and speech, not frozen frames or muted-only tracks. Freeze local/remote publishers, subscribers, visible tiles/preview and screen sender/receiver roles. Screen profiles retain the camera streams above; dropping a camera to share is a different profile. Fix codecs, payload/bitrate settings, layers/simulcast selection, resolution/frame rate, audio channels/rate and hardware/software processing before runs. Each materially different permitted configuration gets a distinct profile and result; do not select a lighter allowed codec/layer after measurement and generalize its pass. Stress adaptation has a separate profile and cannot satisfy an unchanged baseline quality claim.

Every run must compare actual event counts/timestamps, paging/residency and search trace with the manifest; verify negotiated **and delivered** directions, codec/layer, resolution/frame rate, audio activity, screen role and processing path against frozen tolerances. Include per-stream time series and observation gaps so negotiation at startup cannot hide later quality reduction. If an engine/profile cannot supply or validate the specified workload, report unsupported/not measured as appropriate. Never silently widen tolerances or omit failed actions to earn a pass.

Separate GPU live-allocation targets: ≤128 MiB for idle/chat/history, ≤384 MiB for direct video and ≤768 MiB for the conference/screen profile. A tool unable to observe the relevant GPU allocations yields `not measured`, not zero. The reviewer must assess GPU/system-service deltas alongside process footprint before accepting the memory result.

## Q.PERF-03 — Startup and interaction latency

| Scenario | Proposed target and measurement boundary |
| --- | --- |
| Cold startup | Process launch → first interactive native window ≤2 s p95 over 20 runs with a documented cold-cache/reboot procedure |
| Warm startup | Same boundary ≤750 ms p95 over 20 runs; process restarted, OS caches warm |
| Cached room switch | User activation → visible usable cached history ≤100 ms p95, 100 switches |
| Composer feedback | Input event → visible text ≤50 ms p95, ≥1,000 keystrokes |
| Scroll/render | At 60 Hz: frame time ≤16.7 ms p95, ≤33.3 ms p99; no app-caused stall >100 ms during a 60 s scroll trace |
| Message feedback | Send activation → local pending state ≤100 ms p95; server acknowledgement ≤1 s p95 on the baseline network, 100 sends |
| Incoming call | Incoming event received by client → usable incoming-call UI ≤200 ms p95, 30 calls |
| Call connection | Accept/join intent after required permission/consent → first bidirectional audio ≤3 s p95 and remote video ≤5 s p95, 30 calls |
| In-call media | Capture → remote playout: audio ≤200 ms p95 and video ≤300 ms p95 on baseline, using synchronized/instrumented endpoints |

Report authentication/network readiness separately from window startup. Human time spent granting permissions, selecting devices or recording consent is separately reported and excluded from connection setup; instrumented OS/API delay is retained. Keep server-event delivery delay separate from client UI response. Clock synchronization and instrumentation uncertainty must accompany media-latency results; RTT alone is not a one-way latency measurement.

## Q.PERF-04 — Idle, energy and network behavior

No active capture, busy polling or continuous redraw while signed-in idle. Target ≤10 app-attributable timer wakeups/s averaged over the idle run. Hidden/minimized windows must cease unnecessary presentation work; an active call may continue capture/playout intentionally. Switching away from a conversation cancels irrelevant work without losing durable message state.

Idle application traffic after initial synchronization: ≤100 KiB/min total across directions for the 100-room profile, excluding separately reported OS traffic. Respect protocol-prescribed long polling and heartbeats. Failure recovery uses bounded exponential backoff with jitter, observes server retry guidance and stops when its account/session is cancelled. Do not meet a traffic target by missing messages or breaking required keepalive semantics; record a proposed budget adjustment if protocol evidence demands it.

Baseline average media traffic targets, including associated control traffic: audio-only ≤0.2 Mbit/s per direction; direct 720p30 video ≤2.5 Mbit/s per direction; HPB four-person conference ≤3 Mbit/s upload and ≤8 Mbit/s download; shared screen adds ≤3 Mbit/s to the transmitting endpoint and ≤3 Mbit/s to each receiver. Thus the local screen-sender profile targets ≤6 Mbit/s upload / ≤8 Mbit/s download, and the local receiver ≤3 Mbit/s upload / ≤11 Mbit/s download. Record five-second peaks, codec configuration and relay overhead. Internal-signaling peer-to-peer group calls need a separate measured network budget and must not inherit the HPB result.

The [policy-freshness decision](security.md#required-bootstrap-policy-decision) must include refresh requests/responses, retries and timer wakeups in these totals. Before `G.BOOT`, record its per-active-call/host request cadence, worst permitted retry load and estimated bytes/CPU/wakeups against these provisional targets; validate actual cost with the frozen native workload later. Active-call refresh is not silently charged to or exempted from the signed-in idle profile. Coalesce shared sources when their authority and age permit it. If a secure bound cannot fit the budget, independently review a documented budget/profile revision before acceptance measurements; do not lengthen freshness behind the security contract or subtract control traffic from the result. No numeric freshness schedule or measured cost is claimed by this specification.

Under the stress profile, keep local UI latency usable, bound queues and prioritize audible speech; target <1% audio playout underruns in baseline and <3% under stress, with the engine's denominator documented. Publish actual media quality/loss and adaptation, not just network byte counts. No battery-life or competitor advantage claim follows from these targets.

## Q.PERF-05 — Bounded work and resource lifetime

The architecture must specify bounds, cancellation and overload behavior for each expensive path. Virtualize histories/lists; page and evict content instead of materializing total history. Bound decoded-image/GPU caches, media frame queues, search tasks, uploads/downloads and retry queues. Apply backpressure/coalescing to replaceable updates; never silently drop durable messages or required permission/recording transitions. Cancel stale account/room work and reject late results from old generations.

No network, vault, database, media negotiation or unbounded parsing on the UI thread. Size limits and cancellation apply before expensive allocation. Hidden video tiles must follow an explicit receive/decode policy; quality adaptation must be visible in benchmark evidence. Low-memory responses must release disposable caches without discarding unsent work.

Acceptance: inject burst traffic, large histories, slow consumers, cancelled searches/transfers and repeated navigation. Show bounded queue/cache growth and responsive input. Concrete limits belong in module contracts, with a report explaining their effect on these targets.

## Q.PERF-06 — Soak and recovery

After 100 room switches and 20 join/leave cycles, return to the initial idle workload. Within 60 s, footprint must return to within 20 MiB of its initial steady median, with no orphan capture, helpers, sockets or growing task count. A two-hour chat/reconnect soak must show no unexplained monotonic growth; a single final memory sample is insufficient. Document cache warm-up separately from a leak.

Acceptance: preserve raw resource time series, before/after process/task/socket counts and an allocation explanation for retained growth. Exercise sleep/wake, network loss, server restart and account logout while work is in flight.

## Q.PERF-07 — Evidence and profiling

Record exact commit, artifact hash, clean/dirty status, approved workload/profile/fixture hashes, commands, collector versions, raw time series, sample counts, percentiles and worst runs. Include scheduled versus actual workload validation, complete process-lifetime inventory, sample gaps, per-process high-water evidence and provenance, `U` or the qualified alternative peak bound, startup/teardown observations and unmeasured costs. Use release-build runs with the qualified minimum accounting instrumentation for headline metrics; run heavier Instruments/time-profiler/allocation/GPU traces separately to locate costs. Measure accounting/profiler overhead separately; never remove necessary peak/process collection just to describe a run as unprofiled.

Before accepting memory results, qualify the collector with a known touched allocation burst shorter than one second and an owned helper that starts, allocates and exits entirely between normal sample ticks. Also cover helper crash/forced exit, nested/reparented ownership, PID reuse and allocation during startup/teardown. Freeze burst/helper fixture duration/size and scheduling in the qualification manifest; independently observe the injected load. Require lifetime coverage and a peak safeguard that catches an injected ceiling breach even when all regular samples miss it. Record API limitations, measurement uncertainty, collector CPU/memory/timing overhead and comparison runs. Faster periodic sampling alone is not proof of complete high-water coverage. A failed/missing qualification blocks a complete memory pass; its implementation and native runs remain pending.

Acceptance for workload reproducibility: two independent runners reconstruct the same fixtures and normalized planned action/media trace from the frozen manifest, then compare each actual trace/stream profile to its predeclared tolerances. Changed counts, cache residency or quality invalidate that run or require a separately reviewed profile; they cannot inherit the original profile's result. Capture input/render, queue depth, transport, decoding and teardown timing without message contents or secrets.

A Linux VM can verify server behavior and platform-neutral components. It cannot establish macOS Keychain, GPUI rendering, capture, GPU or energy performance. Missing reference hardware is a declared product-verification blocker, not grounds to substitute HTML timings. Publish only sanitized evidence from synthetic accounts, following [security requirements](security.md).

## Q.PERF-08 — Regression and comparison policy

Crossing a budget fails the relevant product gate. Missing process/peak evidence or workload validation makes the result incomplete/invalid and blocks a pass even when reported samples fit. Even within budget, a repeatable >10% regression in p95 footprint or interaction/startup latency requires review before acceptance. Investigate statistically noisy results with an explained repeat; do not discard inconvenient runs without recording the cause.

Comparisons require identical hardware, OS, versions, features, account/history, media quality, topology, duration, approved workload-manifest/profile hash and accounting boundaries. Report absolute values and variability alongside any percentage. A manifest change needs review and a newly measured baseline; old passes do not transfer across materially different activity/media profiles. Different process models, missing media helpers or unequal workloads invalidate a “lighter/faster” conclusion. Bootstrap reviews assess measurability and resource ownership; only native product measurements can pass product budgets.
