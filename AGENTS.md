# Contributor and agent rules

Read [Contributing](CONTRIBUTING.md), [development](docs/development.md) and
[architecture](docs/architecture/README.md). Explicit user instructions govern the
assigned task. Product and security contract changes must be deliberate and visible.

## Engineering

- Give each consequential resource one owner and a small interface. Extend existing
  modules before adding facade layers or generic event buses.
- GPUI views consume bounded snapshots and typed intents. Keep network, credential,
  database and media I/O behind their owners; retain no secrets in presentation.
- Preserve account, conversation, runtime and call scopes. Stale completions cannot
  restore authority or overwrite newer state. Media frames stay out of chat serialization.
- Bound capacity, lifetime, cancellation and overload for queues, caches and asynchronous
  work. Preserve durable intents and required state transitions under overload.
- Model errors and unsupported states honestly; stubs cannot return success. Avoid
  panics on external input and preserve assertions when optimizing.
- Normal sign-in persists accounts, preferring native credential storage with an explicit
  durable **unencrypted private-file** fallback. Preserve [account](docs/product/accounts.md)
  and [security](docs/quality/security.md) contracts; never silently migrate providers.
- Keep focus, keyboard behavior, text input and semantic accessibility distinct. Follow
  [desktop requirements](docs/product/desktop-and-accessibility.md), including en/de/es UI.

## Verification and documentation

Use the pinned toolchain, exact dependencies and lockfile. Choose checks that exercise
changed behavior and failure/lifetime invariants. Linux runtime checks and Windows/macOS
compilation prove different things; neither synthetic fixtures nor compilation establish
native integration, interoperability or performance. Report actual checks and limitations.
Do not request intermediate user testing; prepare the final candidate first.

Keep documentation in English, use NextCloud as the project brand and maintain
[known limitations](docs/limitations.md) when supported behavior changes. Put implementation
rationale beside code and task history in issues, PRs or Git history. Do not add status
ledgers, per-task reports or mandatory ADR churn. Preserve EUPL-1.2, third-party licenses
and provenance. Keep secrets, production data and private environment paths out of public
artifacts. See [SECURITY.md](SECURITY.md) for disclosure.

## Project skills

Use `gpui-guidelines` for GPUI lifetime/rendering/focus/input work, `gpui-test` for
deterministic GPUI test diagnosis and `gpui-bench` for performance measurements.
