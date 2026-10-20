# Contributor and agent rules

Start with [contributing](CONTRIBUTING.md), the [product contracts](docs/product/README.md), [quality requirements](docs/quality/index.md) and [architecture](docs/architecture/README.md). Explicit user decisions govern the assigned work; surface conflicts with published contracts instead of silently weakening them.

## Engineering

- Give each consequential policy or resource one cohesive owner and a small interface. Keep protocol details, credential storage, persistence transactions, timers and media internals behind that owner. Extend an existing logical module before adding facade layers, generic event buses or feature-per-crate scaffolding.
- GPUI presentation consumes bounded snapshots and typed intents/results/errors. Views perform no network, credential-store, database or media I/O and retain no secrets. Keep account, host, conversation, runtime and call scopes explicit; an obsolete completion cannot confer authority or overwrite a newer state. Media frames stay out of chat/event serialization.
- Specify finite capacity, lifetime, cancellation and overload behavior for every queue, cache and asynchronous operation. Bound work before allocation; page and virtualize collections. Cancellation must account for ongoing effects and cleanup, not merely drop a UI handle. Preserve durable work and required state transitions under overload.
- Model meaningful states and error outcomes explicitly. Propagate actionable failures to the owning layer with safe context; handle expected cancellation deliberately. Avoid panics on external input and unchecked indexing. Unsupported or unconfigured paths must say so; stubs cannot return success. Keep correctness assertions and required behavior intact when optimizing.
- Prefer native OS credential stores, with an explicit durable **unencrypted**, private per-user file fallback. Normal sign-in must persist the account, as specified in [accounts](docs/product/accounts.md). Preserve access controls and honest recovery/cleanup outcomes; do not replace this requirement with mandatory application encryption or OS-only persistence.
- Treat visible focus, keyboard behavior, text input and semantic accessibility as separate obligations. Follow [desktop requirements](docs/product/desktop-and-accessibility.md), including English, German and Spanish UI; a styled control or toolkit experiment does not establish accessible product behavior.

## Scope and verification

- Use the [development guide](docs/development.md) for prerequisites and scoped commands. Respect the selected toolchain, dependency/feature pins and architecture checks; do not alter them to make an example pass.
- Choose checks that exercise changed behavior and failure/lifetime invariants. Start with the smallest relevant target, use deterministic reproduction, and broaden only for shared impact or unresolved risk. Trivial reversible edits do not need tests that restate their implementation. Report exact inputs, commands, outcomes and unrun checks.
- Keep Linux runtime tests distinct from Windows/macOS compile-only CI checks. Compilation, synthetic fixtures and software rendering do not qualify native integrations, accessibility or performance. Final user testing is reserved for the final prepared candidate after the assigned agent-side work; do not ask the user to test intermediate increments.
- Use proportionate checks and follow the explicit task owner’s review scope under the [verification contract](docs/quality/verification.md). Routine documentation or settings work does not automatically require another review loop. Author checks alone do not authorize product/release claims. Handoffs identify changed behavior, source identity, affected contracts, evidence limits and remaining work.

## Documentation and public hygiene

- When behavior changes, maintain the relevant user/developer contracts. Specifications describe intended behavior and acceptance; high-level architecture describes responsibilities, boundaries and data flow. Record concise actual limitations in [current state](docs/status.md), without duplicating a project diary in contributor rules.
- Put nonobvious implementation reasons, invariants and tradeoffs beside the relevant code or in its doc comments. Preserve useful API documentation and comments; avoid separate implementation-decision documents and mandatory status/ADR churn for every internal detail. Keep needed historical evidence in durable private storage outside normal contributor documentation; public docs are not a task ledger or report archive.
- Write public documentation in English. Use NextCloud as the project brand and describe the client in its team/company context. Preserve the project's [EUPL-1.2 license](LICENSE) and third-party terms/provenance. Keep secrets, production data, private environment paths and sensitive raw traces out of public artifacts; use synthetic fixtures and follow [SECURITY.md](SECURITY.md).

## Project skills

- `gpui-guidelines`: use when implementing or reviewing GPUI lifetime, rendering, collections, focus and input behavior.
- `gpui-test`: use when designing or diagnosing deterministic GPUI tests, scheduler seeds, timers and pending work.
- `gpui-bench`: use when designing or interpreting bounded GPUI performance measurements and comparisons.