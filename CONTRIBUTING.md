# Contributing

NextCloud Modern Talk is an independent Rust/GPUI client licensed under EUPL-1.2.
Start with [development](docs/development.md), [architecture](docs/architecture/README.md)
and [known limitations](docs/limitations.md).

## Issues and pull requests

For a bug, include the app version, OS, relevant NextCloud/Talk versions, reproduction
steps, expected behavior and actual result. Use synthetic data and remove credentials,
private server names and message contents. Report vulnerabilities through [SECURITY.md](SECURITY.md).
For a substantial feature, open an issue first to agree on scope and supported platforms.

Keep changes focused. Preserve account/room/call isolation and keep I/O behind its owning
module. [Product requirements](docs/product/README.md) and [security requirements](docs/quality/security.md)
define behavior; link relevant requirements when changing it. Do not treat compilation
or a mocked response as proof of real-server or native-platform support.

## Checks

Run the checks appropriate to the change; the [development guide](docs/development.md#bootstrap-checks)
lists scoped commands. Typical commands are `task fmt:check`, `task lint`, `task test`,
`task docs` and `python3 tools/check_architecture.py`. Use the pinned Rust toolchain and
`--locked`. Dependency updates must update the lockfile and preserve explicit feature
selection, source provenance and third-party notices.

Describe what changed, checks run and any untested platforms or remaining limitations
in the PR. Runtime, mock and compile-only results are different evidence. CI runs Linux
tests and native smoke checks, plus Windows/macOS builds; see [CI policy](docs/ci.md).

## Documentation and licensing

Write public documentation in English and use NextCloud as the project brand. Keep
installation, development and limitations current. Put implementation rationale beside
code; use issues, PRs and Git history for task status and historical reports.

Contribute only material you can distribute under the applicable terms. Preserve
[EUPL-1.2](LICENSE), [third-party notices](NOTICE.md), asset licenses and source provenance.
