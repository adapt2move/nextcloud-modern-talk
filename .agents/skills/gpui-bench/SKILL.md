---
name: gpui-bench
description: Design, review and interpret bounded GPUI performance probes using production paths, fixed workloads and honest latency, completion and memory evidence. Use for a performance question or comparison, not ordinary correctness tests.
---

# GPUI performance measurements

Apply [AGENTS.md](../../../AGENTS.md), current execution constraints and the [performance requirements](../../../docs/quality/performance.md). Derive budgets and acceptance workloads from those requirements and the [measurement contract](../../../docs/architecture/measurement.md); never invent a new ceiling or shorten an acceptance workload to obtain a pass. A short diagnostic probe is separate evidence.

## Establish what is being measured

Name the user operation, production entry point, completion signal and competing UI work. Fix input sizes, generator/seed or fixture hashes, event cadence, loaded-page limits, expected counts/order and cache state. Identify the candidate, baseline, toolchain, features, OS, CPU/GPU/driver, display and software-rendering status. Change one workload dimension at a time.

Separate preparation/reset, timed work and validation. Keep setup outside the interval unless startup/setup is the subject. Validate actual completed work and final state; dropped events or lighter media are not speedups. For responsiveness, record input-to-result or instrumented frame boundaries while useful work competes with the measured path, plus total completion time and workload counts. Enqueue time is not completion; a paint callback or GPU submission is not display presentation.

## A path supported by GPUI 0.2.2

The project pins GPUI **0.2.2**, Rust **1.99.1** and an ordinary Cargo release profile. It has no `gpui::bench`, `BenchAppContext`, `bench-support`, `BenchMeasurement` or `CountingAllocator`; `release-fast` is not a project profile. Do not introduce these names or upgrade dependencies to make Zed main examples work. [Source/API references](references/sources.md) distinguish supported primitives from upstream-only helpers.

Use a small ordinary executable probe that calls the production implementation. When a task authorizes a new harness, an autodiscovered Cargo example is an option without a new benchmark framework. Reuse the real implementation through the owning module's permitted seam; do not copy its algorithm into the probe. If no seam exists, report the required scoped instrumentation change instead of silently widening public APIs or bypassing the architecture guard.

- For synchronous work, use `std::time::Instant` around the bounded operation and `std::hint::black_box` where needed to inhibit optimization. Assert results outside the interval and retain raw samples. Do not substitute a no-op output to improve timing.
- For GPUI scheduling/window work, use `Application::new().run(...)` on the normal event-loop thread, the production `App`/`Context` and their executors. `cx.background_executor().spawn(...)` handles eligible background computation; the foreground applies results through the real update path. Keep task ownership and await/observe actual completion. Use an explicit finite action count, completion/failure record and orderly `App::quit` after cleanup.
- Keep `TestAppContext`, fake time and `test-support` out of this measurement graph. Check the selected target's resolved features, including applicable dev dependencies, before building; ordinary examples can also activate dev dependencies. Record that graph. A diagnostic result with doubles needs its own limited scope, not a production performance label.

After the probe exists and execution is authorized, these are **templates**, not installed targets:

```sh
cargo build --locked --release -p <owning-package> --example <probe-name>
timeout --signal=TERM --kill-after=5s 60s <built-probe-path>
```

Replace every placeholder with the selected package, example and actual artifact path; account for target-directory/target-triple settings. `talk-desktop` is the existing presentation package. Its normal binary can be built with `cargo build --locked --release -p talk-desktop --bin talk-desktop`, but launching a fixture window alone is not a benchmark harness. The commands and API sketches here have not been compiled or run as part of skill authoring.

The timeout example assumes GNU `timeout` on an authorized Linux environment and bounds a short probe, not compilation or acceptance workloads. Verify the installed tool or use an equivalent process-group watchdog. A terminal tool returning a session ID is not a timeout. Keep a finite outer deadline even with an internal watchdog; the latter can stall with the event loop. Preserve timeout/failure results, terminate owned descendants and verify cleanup. Longer authorized workloads keep their prescribed durations and an explicit finite deadline.

## Measure and report

Build before timing. Smoke-check correctness and termination, then run one measured process at a time on otherwise idle resources. Disclose cold/warm process, OS-cache, glyph/image-cache and fixture state. Use the same probe, inputs, build flags and machine conditions for baseline and candidate; disclose dirty inputs and instrumentation overhead. Run heavier profiling separately.

Report raw sample count, units, measurement boundaries, distributions/ranges and worst observations alongside completion counts. Include process memory observations with collector, units, lifetime coverage and missing processes. Linux RSS/high-water RSS or allocator bytes are diagnostic metrics; they cannot substitute for the macOS footprint/peak accounting in Q.PERF. Report unavailable GPU, energy or hardware counters as unmeasured; do not require extra privilege or report zero. Small samples do not justify strong tail-percentile claims.

Retain all raw runs, correctness failures, timeouts and noise explanations. Repeat only to answer a documented uncertainty, keeping the original result. Xvfb, software Vulkan, synthetic inputs, headless work and compile-only Windows/macOS CI do not establish hardware GPU, Mac, accessibility or full-app qualification. Follow the final-only user Mac testing constraint; do not move intermediate benchmarking onto the user.
