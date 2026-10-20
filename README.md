# NextCloud Modern Talk

A native desktop client for teams using NextCloud Talk, built in Rust with GPUI and no WebView. It brings conversations, messages, files and calls together, with Manrope typography, blue and sage accents, and system, light and dark appearance.

The target server families are **NextCloud 34 / Talk 24** and **NextCloud 35 / Talk 25**. Availability depends on server capabilities, configuration and permissions. macOS is the first product target; Linux is a development and packaging target.

**v0.0.2 alpha:** browser-based account connection, conversation settings, messaging, background activity, audio/video calls and automatic update downloads. The native UI uses Lucide icons, contextual chat actions and subtle transitions that respect Reduce Motion. Download the [alpha release](https://github.com/adapt2move/nextcloud-modern-talk/releases/tag/v0.0.2) and read its [installation instructions and limits](docs/alpha.md). Full official-client parity is still in progress.

![Native conversation view in German, showing a synthetic team conversation](docs/assets/chat.png)

The untouched screenshot shows the native Linux app with fictional conversations in its built-in UI scenario.

## Contribute

Start with [Contributing](CONTRIBUTING.md) and the [development guide](docs/development.md) for prerequisites, the pinned toolchain and scoped commands. UI scenarios use fictional data and do not log into a server. The [backend lab](docs/backend-lab.md) uses Docker and synthetic accounts.

## Documentation

- [Product requirements](docs/product/README.md) and [feature inventory](docs/compatibility/feature-inventory.md)
- [Architecture](docs/architecture/README.md) and [design reference](docs/design/README.md)
- [CI policy](docs/ci.md) and [release tooling](tools/release/README.md)
- [Documentation index](docs/README.md) and [security reporting](SECURITY.md)

Licensed under [EUPL-1.2](LICENSE); third-party assets retain their [notices and licenses](NOTICE.md). This is an independent project, not an official NextCloud client.
