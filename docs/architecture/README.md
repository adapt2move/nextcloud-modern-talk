# Architecture bootstrap

Status: **candidate for `G.BOOT`; not accepted and not a functioning client**. [IST](../status.md) is authoritative for implementation state. The accepted product/quality baseline remains unchanged. These decisions assign the mandatory [bootstrap obligations](../reviews/product-spec-review-r2.md#mandatory-bootstrap-handoff) to concrete owners; their native implementations and proofs are future work.

| ID | Decision and contract |
| --- | --- |
| [ADR-001](decisions/001-native-ui.md) | Rust/GPUI native direction, toolchain and dependency/license boundaries |
| [ADR-002](decisions/002-deep-modules.md) | Three packages; cohesive modules, narrow interfaces, dependency direction and resource ownership |
| [ADR-003](decisions/003-credential-lifecycle.md) | Distinct generations, durable nonsecret restoration, staging/settlement/crash recovery and account rejection |
| [ADR-004](decisions/004-policy-media-lifecycle.md) | Authoritative policy mapping, numeric freshness/control costs, local media enforcement and shutdown |
| [Measurement contract](measurement.md) | Executable workload/validator ownership and complete process/peak evidence qualification |
| [Source evidence](source-evidence.md) | Official source pins, observations and limits of feasibility research |

## Validation

From the repository root, the designated verifier runs these on the assigned development VM (or the documented CI runner). Static source authors must not start a VM or run these on the host when their brief prohibits it. `agent-dev-box exec --` is the assigned-session transport; it does not grant provisioning ownership.

```sh
python3 tools/check_architecture.py
python3 tools/check_docs.py
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo run --locked -p talk-bootstrap
```

Python requires 3.11+ (`tomllib`); Rust is pinned in [rust-toolchain.toml](../../rust-toolchain.toml). There are no registry dependencies. [Cargo.lock](../../Cargo.lock) explicitly lists only the local graph; the statically authored bytes passed the integrator's pinned-Cargo locked checks without regeneration. [IST](../status.md) records preliminary execution and the remaining documentation failure; independent validation is still required. If Cargo requests regeneration in a later candidate, stop that validation attempt: the integrator runs `cargo generate-lockfile --offline` on the VM, records the candidate diff and requests fresh locked checks. Do not silently mutate the candidate during verification. Format corrections similarly require an explicitly recorded integration step with `cargo fmt --all` and follow-up checks.

`check_architecture.py` enforces the exact dependency/lock graph, inherited unsafe policy, absence of hidden build/platform edges, and the inspected renderer/core restrictions. It is a conservative source guard, not a Rust parser, information-flow proof or general sandbox. Independent verifier negative fixtures should add an adapter dependency to core, introduce a target-specific edge, import an adapter into presentation, expose an unbounded renderer field and insert a raw OCS route into core; each must fail. Restore each fixture before subsequent checks. Governance owns the separate docs checker and CI.

Rust tests exercise account isolation/stale completions, narrow rejection classification, conservative lease expiry/clock epochs, gate conjunction, exact-generation cleanup/restoration and UTF-8/page bounds. The composition diagnostic must print its unconfigured state and exit; that is not evidence of login, GUI or media. Record commands, platform, exact tree/patch, results and limitations under [Q.VER-03](../quality/verification.md#qver-03--exact-candidate-and-evidence-identity).

## Qualification boundaries

- Both target families have candidate source mappings. Neither currently has a supported native runtime profile. `S=2 s` is a deployment qualification requirement, not an upstream guarantee. Missing/unbounded required sources block affected paths; they do not grant permissions by version inference.
- GPUI/media/SQLite/native-vault dependencies are not selected into the lockfile. Adding them requires reviewed pins, licenses, platform feasibility and affected ADR/IST updates after `G.BOOT` acceptance.
- Metadata durability, post-crash vault writer settlement, continuous clock/sleep behavior, process termination/device release, HPB/internal interoperability and E2EE require native/integration evidence. A missing recovery barrier retains uncertainty.
- No memory, startup, energy, capture-release or policy-latency benchmark has been run. The required workload and collector designs do not assert feasibility of the provisional budgets.

The PM must obtain separate architecture/module, functional/compatibility, performance and coarse-security reviews, resolve objections and accept the exact candidate before full product implementation. This author does not accept `G.BOOT`.
