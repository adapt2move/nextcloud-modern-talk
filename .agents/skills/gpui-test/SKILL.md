---
name: gpui-test
description: Design and diagnose deterministic GPUI tests, fixed-seed failures, controlled timers and task lifetimes against this project's published GPUI dependency. Use for GPUI test work, not native rendering or performance qualification.
---

# Deterministic GPUI tests

Follow [AGENTS.md](../../../AGENTS.md) and the [development guide](../../../docs/development.md) before execution. This skill does not lift execution restrictions. API facts below were checked against GPUI and gpui-macros **0.2.2**; the project toolchain is **Rust 1.99.1**. Recheck the manifests if these pins change. [Sources](references/sources.md) identify the exact implementations.

## Select the right test seam

Use an ordinary Rust test for pure state transitions. Use `#[gpui::test]` when entity contexts, scheduler interleavings or controlled time are part of the invariant. Assert outcomes such as rejecting stale room results, preserving drafts, releasing subscriptions or bounding admitted work, rather than merely constructing a view. Do not add tests that mirror trivial reversible edits.

The macro is exported in 0.2.2, but its generated harness and `TestAppContext` require GPUI's `test-support` feature for downstream packages. The current `talk-desktop` manifest does **not** enable it. That feature also adds dependencies/features, including Wayland; a downstream `#[cfg(test)]` does not enable GPUI's own test configuration. Do not change manifests, lockfiles or guards implicitly. A future harness needs an explicitly scoped dependency decision; until then, report this prerequisite and use existing ordinary tests where appropriate.

## Reproduce narrowly

Select the package, target and fully qualified test name from the actual candidate. For an existing desktop binary unit test, after execution is authorized:

```sh
cargo test --locked -p talk-desktop --bin talk-desktop -- --list
cargo test --locked -p talk-desktop --bin talk-desktop 'module::test_name' -- --exact --nocapture
```

`module::test_name` is a placeholder: replace it with a listed name and check that the intended test ran. Zero matched tests are not a pass. These are source-checked command templates, not executed evidence.

For a package/target with a verified GPUI test harness, use these prefixes with the selected command:

- `SEED=17 ITERATIONS=1`: reproduce seed 17 exactly; substitute the observed failure seed. An inherited iteration count must not turn a reproduction into a sweep.
- `ITERATIONS=20` with `SEED` unset: a small interleaving sweep after the fixed seed passes. Include the attribute's explicit seeds in the actual run count.
- `RUST_BACKTRACE=1`: retain the panic context alongside the seed, input and terminal output.

The 0.2.2 macro accepts `seed = N`, `seeds(...)`, `iterations = N`, `retries = N` and `on_failure = "path::function"`. Environment `ITERATIONS` overrides the attribute; `SEED` supersedes explicit seeds. **Version quirk:** with `SEED=N` and iterations greater than one, 0.2.2 runs N once and then N..N+iterations, repeating N. Do not promise that many distinct executions. Keep retries at zero for regression diagnosis; a retry that eventually passes must not conceal the original failure.

Both sync and async functions accept references to `TestAppContext`; async tests additionally accept `BackgroundExecutor`, while references to `App` are sync-only. `StdRng` is recognized by either form, but generated code names `rand` directly: verify a compatible direct test dependency before using it. Do not add dependencies just to imitate upstream examples.

## Control time and diagnose lifetime failures

- In `TestAppContext`, use `cx.background_executor.timer(duration)` and `cx.background_executor.advance_clock(duration)`, then `cx.run_until_parked()` as appropriate. In an entity context the executor accessor is `cx.background_executor()`. Keep awaited timers on this dispatcher; wall-clock sleeps, foreign timers and external runtimes are outside its control.
- Pump until ready work is drained, then assert the intended completion or pending state. Draining ready work alone does not prove every task has completed. In 0.2.2, clock advancement can also pump work; inspect ordering when testing debounce/cancellation.
- For `parked with nothing left to run`, trace the awaited receiver, timer and task owner. `BackgroundExecutor::set_waiting_hint`, `start_waiting` and `finish_waiting` provide version-supported diagnostics. Keep hints nonsecret and clear them after the wait. Inspect dropped tasks, retained entity cycles, closed channels and escaped executors before changing timing.
- `PENDING_TRACES`, `GPUI_RUN_UNTIL_PARKED_LOG`, `DEBUG_SCHEDULER` and `SCHEDULER_NONINTERACTIVE` from newer guidance are not implemented in these published GPUI sources. Do not claim they activate traces. Do not detach tasks, allow parking indefinitely, increase retries or remove assertions just to make teardown pass.

Keep the seed and sensitive await/yield ordering while minimizing a failure. After a fix, rerun that case and a bounded useful sweep, then only related regression targets. Record omissions. Deterministic test windows do not establish actual GPU presentation, OS text/IME behavior, semantic accessibility, native credentials or full-app performance; those require separately authorized evidence.
