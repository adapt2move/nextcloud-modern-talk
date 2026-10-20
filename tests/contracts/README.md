# Architecture boundary regressions

Run on the assigned development VM or CI runner, with the pinned toolchain available:

```sh
python3 tests/contracts/run_boundary_fixtures.py --candidate . --output /tmp/identity-ui-boundary-fixtures.json
```

The runner copies Cargo inputs, source, diagnostic bridge and the architecture checker to temporary directories and removes them and its external target on exit. It never mutates the candidate. It checks bounded presentation use through `talk_core::ui`, nested grouped imports, the original five forbidden-edge/container/route cases, the exact BFR1-01 BTreeMap reproduction and direct collection/worker/secret/frame regressions. Each negative must match its intended diagnostic. JSON records command output and unchanged lockfiles. Recursive core UI and native presentation negatives cover nested routes, protocol imports, hidden network calls and clipboard access outside input.rs. Positive consumer compilation targets the cheap headless package; native GPUI compilation is a separate required check with platform prerequisites. These are compile/lexical guard checks, not a Rust sandbox or native product evidence.

`cargo test --workspace --locked` runs the current core/adapter/native behavioral tests. Default-off real-backend tests require explicit fixtures and are ignored otherwise. Synthetic identity receipts establish correlation only; no journal, vault, provider settlement or auth recovery is simulated as successful. Keep needed historical evidence outside normal contributor documentation.
