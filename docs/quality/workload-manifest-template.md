# Performance workload manifest — TEMPLATE, not a frozen workload

Required by [Q.PERF-02](performance.md#qperf-02--workload-budgets), [Q.PERF-07](performance.md#qperf-07--evidence-and-profiling) and [Q.LAB-05](test-servers.md#qlab-05--reproducibility-evidence). Replace every placeholder and freeze the complete manifest, referenced input files and independent review **before** acceptance measurements. A seed, codec name or after-the-run description alone is insufficient. Store only synthetic data and nonsecret logical endpoint/account labels.

## Identity and review

- Manifest ID/revision and schema; workload budget row and distinct media/processing profile: `<...>`
- Owner; independent reviewer; approval date/evidence; manifest and referenced-input SHA-256s: `<...>`
- Exact fixture/replay/validation tool revisions; deterministic generator, PRNG, seed and corpus hashes: `<...>`
- Client build identity and reference hardware/OS; server/peer/image/package and topology manifest: `<links>`
- Planned warm-up, steady interval, repeats, startup/teardown boundaries and reset between runs: `<...>`

Freeze and hash the manifest payload using a declared canonicalization rule; keep its own digest/approval in a sidecar or review record to avoid a self-referential hash. Reusing the workload with a new client candidate requires the candidate identity in the run record; do not rewrite the frozen input manifest. Schema, profiles, replay and validators are bootstrap obligations, not delivered tools in this template.

## Activity and data inputs

| Required input | Frozen specification |
| --- | --- |
| Accounts/rooms/history | `<counts, room distribution, selected room, corpus content/length definition, replies/reactions/attachment overlap, avatars/thumbnails and hashes>` |
| Initial and warm-up state | `<sync checkpoint, loaded/resident pages and page size, disk/memory/search/image cache state, cache limits/eviction policy, warm-up actions and exact reset procedure>` |
| Messages and typing | `<relative timestamp trace, counts/rates/bursts, incoming/outgoing split, payload IDs, typed keystrokes/cadence, send/ack expectations and failure rule>` |
| Navigation and scrolling | `<room IDs, switch cadence/count, scroll direction/velocity/positions/duration, page request sequence/count and residency checkpoints>` |
| Search | `<exact queries, hit/miss and broad/narrow mix, cached/uncached conditions, timing/cancellation, expected result sets/pages>` |
| Display and background work | `<window/display scale, visible tiles, preview/layout, focus/hidden state, animations, concurrent transfers and policy-refresh schedule>` |
| Tolerances | `<expected totals, allowed timing jitter/count deviations, residency bounds, measurement gaps and explicit invalidation rules; no unbounded or “representative” fields>` |

Include a normalized planned input trace hash. The active-chat summary requires 200 messages in ten steady minutes; fill in the schedule and other actions. Large-history paging/scrolling/search needs its own complete trace; “continuous” is not a cadence. Planned fault/stress profiles must name their expected failures separately.

## Media inputs

| Required input | Frozen specification |
| --- | --- |
| Every endpoint and stream | `<logical publisher/subscribers, audio/camera/screen direction, self-preview, visible/decoded tiles, local screen sender or receiver role>` |
| Source assets | `<moving camera/screen clips and speech hashes, timeline/loop, motion/audio activity, source capture method and generator revision>` |
| Codec and transport profile | `<codec/profile, payload/bitrate/packetization, layers/simulcast subscriptions, HPB/internal signaling and direct/TURN, hardware/software encode/decode>` |
| Delivered quality | `<resolution, frame rate, audio channels/rate, minimum sustained activity, acceptable loss/adaptation and observation interval/gaps>` |
| Allowed variants | `<separate profile IDs/hashes and expected budgets for each materially different codec/layer/processing path; no result pooling>` |

Keep all camera/audio streams when adding a screen in the baseline conference profiles. A lighter direction/layout/layer or a stress-induced reduction cannot count as that baseline. Define per-stream quality tolerances before collection, including how long a deviation may last; negotiation at join alone cannot verify delivery throughout a run.

## Collection and validation

- Collector/version and qualified API/structure revision; overhead/error limits and qualification evidence: `<...>`
- Lifetime process discovery, birth/exit identity, high-water provenance including abnormal exit; aggregate samples, `U` or qualified tighter peak bound: `<...>`
- Independent collector qualification inputs: subsecond touched allocation, between-tick helper and crash/startup/teardown cases, expected breach and fixture hashes: `<...>`
- Actual activity evidence: planned/issued/completed counts and relative times, paging/residency checkpoints, query trace, missed actions and invalidation result: `<...>`
- Actual media evidence: negotiated and delivered directions, per-stream quality/codec/layer/processing time series, screen role, adaptation and gap assessment: `<...>`
- Independent reconstruction: two runners, matching fixture/planned-trace hashes and actual-profile validator results: `<...>`
- Per-run outcome: `<valid workload / invalid workload; complete / incomplete accounting; measured pass/fail only when prerequisites hold>`

Keep actual results in separately hashed run records referring to the frozen manifest. A divergence is evidence to investigate; revision requires a new review/profile and applicable reruns, not retroactive edits to the accepted inputs.
