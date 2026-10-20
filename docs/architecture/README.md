# Architecture

The independent [NextCloud Talk API](../../crates/nextcloud-talk-api/README.md) owns origin-bound transport and typed protocol values, including the meeting/signaling seam. It has no dependency on application policy or native media. The app has three layers: a bounded core, cohesive adapters for consequential I/O and policy decisions, and native GPUI presentation/composition. Views consume typed snapshots and intents; they do not own networking, credentials, persistence or media. The [alpha scope](../alpha.md) distinguishes connected functionality from remaining compatibility and platform work.

| Contract | Responsibility |
| --- | --- |
| [ADR-001](decisions/001-native-ui.md) | Rust/GPUI native direction, toolchain and dependency boundaries |
| [ADR-002](decisions/002-deep-modules.md) | Cohesive modules, narrow interfaces, dependency direction and resource ownership |
| [ADR-003](decisions/003-credential-lifecycle.md) | Generations, durable restoration, settlement/recovery and account rejection |
| [ADR-004](decisions/004-policy-media-lifecycle.md) | Policy mapping, freshness, local enforcement and shutdown |
| [ADR-005](decisions/005-native-media.md) | Maintained WebRTC engine, device ownership and distribution |
| [ADR-006](decisions/006-credential-providers.md) | Native/private-file providers and qualification boundaries |
| [ADR-007](decisions/007-nextcloud-transport.md) | Bounded HTTP/TLS/OCS owner and diagnostic signaling |
| [Integration contract](nextcloud-contract.md) | Construction, cancellation, lifetime and projection obligations |
| [Identity contract](identity-contract.md) | Private runtime and storage transaction boundaries; durable owner, cancellation and restoration |
| [Media allocation](media-allocation.md) | Future allocation targets and unresolved native/GPU accounting |
| [Measurement](measurement.md) | Workload ownership, process/lifetime/peak evidence and collector qualification |
| [Source provenance](source-evidence.md) | Official source pins and limits of feasibility research |
| [Credential sources](credential-provider-sources.json) | Upstream pins and provenance; not runtime qualification |

## Validation

Use the scoped commands and prerequisites in [development](../development.md#bootstrap-checks). Keep the pinned toolchain and lock; never regenerate dependencies to make a check pass.

[check_architecture.py](../../tools/check_architecture.py) guards dependency/lock boundaries, unsafe policy and renderer/core restrictions. It is a conservative lexical guard, not a Rust parser or general sandbox. [Boundary fixtures](../../tests/contracts/README.md) must distinguish intended diagnostic failures from incidental failures. Static checks, compilation and synthetic tests cannot establish native behavior or product support.

## Qualification boundaries

- Both server families need real integration evidence for each claimed profile. Missing or unbounded policy sources block the affected path; version numbers do not grant permission.
- The production engine runs in the ordinary account/call owner. The old IPC helper is diagnostic-only; its tests do not qualify production device authority or native allocation bounds.
- Credential durability, crash recovery, clock/suspend behavior, process/device release, native rendering, HPB/TURN and E2EE require their own evidence. Uncertainty must remain visible.
- No combined-app startup, memory, energy, capture-release or policy-latency benchmark is established. Required workloads and collectors are specifications, not measured passes.

Keep implementation rationale and local invariants beside the owning code. Historical execution reports belong outside normal contributor documentation.
