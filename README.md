# Nextcloud Modern Talk

A planned native Nextcloud Talk client in Rust and GPUI, with macOS as the first platform. The goal is conversation, chat, files and calls in one desktop experience, compatible with Nextcloud 34 / Talk 24 and Nextcloud 35 / Talk 25 through capability, configuration and permission checks.

**This repository is a reviewed specification and contract bootstrap, not a usable Talk client.** The Rust workspace has typed boundaries and unconfigured adapters. It has no GPUI renderer, authenticated network integration, OS vault, native media engine or application runtime. All 140 product features remain **Planned**. See the [current state](docs/status.md).

Published as **work in progress**. The [PM acceptance record](docs/reviews/bootstrap-acceptance.md) accepts `G.BOOT` for exact source `54cde8e317ed0ea98d88255b9f12cb9a1151229b`, after four independent round-2 reports passed with no exceptions. This publication adds those reports, the decision and current-state prose; it does not implement product features. `G.PRODUCT` and `G.RELEASE` have not been reached. Native functionality, security, media interoperability and performance remain unverified.

- [Product specifications](docs/product/README.md) and [feature inventory](docs/compatibility/feature-inventory.md)
- [Design reference](docs/design/README.md) and [HTML design gallery](docs/design/prototypes/index.html) — illustrations, not native integration
- [Architecture and bootstrap decisions](docs/architecture/README.md)
- [Quality and acceptance gates](docs/quality/index.md), [documentation index](docs/README.md)
- [Contributing](CONTRIBUTING.md), [agent rules](AGENTS.md), [security](SECURITY.md)

The workspace pins Rust in [rust-toolchain.toml](rust-toolchain.toml) and currently uses only the standard library and local packages. The headless diagnostic `cargo run --locked -p talk-bootstrap` prints its unconfigured state. [Validation commands](docs/architecture/README.md#validation) and evidence requirements distinguish a compiling scaffold from product verification.

Licensed under [EUPL-1.2](LICENSE); see [NOTICE.md](NOTICE.md) for asset and third-party notices. This is an independent project, not an official Nextcloud client.
