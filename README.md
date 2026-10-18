# Nextcloud Modern Talk

A planned native Nextcloud Talk client in Rust and GPUI, with macOS as the first platform. The goal is conversation, chat, files and calls in one desktop experience, compatible with Nextcloud 34 / Talk 24 and Nextcloud 35 / Talk 25 through capability, configuration and permission checks.

**This repository is a specification and contract bootstrap, not a usable Talk client.** The Rust workspace has typed boundaries and unconfigured adapters. It has no GPUI renderer, authenticated network integration, OS vault, native media engine or application runtime. All 140 product features remain **Planned**. See the [current state and pending reviews](docs/status.md).

Published as **work in progress**. All four independent bootstrap review disciplines require changes; corrections are in progress, with independent recheck and PM acceptance pending. `G.BOOT` is **not accepted**. Public source availability is not a release or gate approval.

- [Product specifications](docs/product/README.md) and [feature inventory](docs/compatibility/feature-inventory.md)
- [Design reference](docs/design/README.md) and [HTML design gallery](docs/design/prototypes/index.html) — illustrations, not native integration
- [Architecture and bootstrap decisions](docs/architecture/README.md)
- [Quality and acceptance gates](docs/quality/index.md), [documentation index](docs/README.md)
- [Contributing](CONTRIBUTING.md), [agent rules](AGENTS.md), [security](SECURITY.md)

The workspace pins Rust in [rust-toolchain.toml](rust-toolchain.toml) and currently uses only the standard library and local packages. The headless diagnostic `cargo run --locked -p talk-bootstrap` prints its unconfigured state. [Validation commands](docs/architecture/README.md#validation) and evidence requirements distinguish a compiling scaffold from product verification.

Licensed under [EUPL-1.2](LICENSE); see [NOTICE.md](NOTICE.md) for asset and third-party notices. This is an independent project, not an official Nextcloud client.
