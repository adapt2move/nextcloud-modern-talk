# Bootstrap boundary regressions

Run on the assigned development VM or CI runner, with the pinned toolchain available:

```sh
python3 tests/contracts/run_boundary_fixtures.py --candidate . --output /tmp/identity-ui-boundary-fixtures.json
```

The runner copies only Cargo inputs, source and the architecture checker to temporary directories and removes them and its external target on exit. It never mutates the candidate. It checks bounded presentation use through `talk_core::ui`, nested grouped imports, the original five forbidden-edge/container/route cases, the exact BFR1-01 BTreeMap reproduction and direct collection/worker/secret/frame regressions. Each negative must match its intended diagnostic. JSON records command output and unchanged lockfiles. These are compile/lexical guard checks, not a Rust sandbox or native product evidence.

`cargo test --workspace --locked` also runs the original seven core contracts, six new identity predicate cases, one presentation case and two concrete unconfigured-identity cases. Synthetic receipt records establish correlation only; no journal, vault, runtime, provider settlement or auth recovery is simulated as successful. Historical independent probes and evidence stay immutable under `docs/reviews/evidence/`.
