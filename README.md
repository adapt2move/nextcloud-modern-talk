# NextCloud Modern Talk

A native desktop client for NextCloud Talk, built in Rust with GPUI and no WebView.
Keep team conversations, shared files and calls in one workspace.

- A focused sidebar with search, Activity, favorites and conversation menus.
- Messages, threads, reactions, polls, voice messages and file browsing.
- Audio/video calls and screen sharing, subject to server policy and the supported media profile.
- Multiple accounts, bounded offline history, drafts and queued outgoing changes.
- English, German and Spanish UI, light/dark appearance, and blue or sage accents.

Targets **NextCloud 34 / Talk 24** and **NextCloud 35 / Talk 25**. Features depend on
server capabilities, configuration and permissions. **macOS comes first** (Apple
Silicon); Linux supports development and packaging. Windows is compile-only.
This is alpha software; see [known limitations](docs/limitations.md).

**[Download an alpha release](https://github.com/adapt2move/nextcloud-modern-talk/releases)**
and follow the [installation guide](docs/install.md) for requirements, first launch
and updates. The macOS DMG includes its media runtime.

![Native chat with the Activity sidebar and fictional German team messages](docs/assets/chat.png)

Linux/X11 capture of the compiled app at 1280×720, light appearance with the blue
accent and German UI. The `chat` scenario uses fictional team data and is not
connected to a server.

## Contribute

Read [Contributing](CONTRIBUTING.md) and [development](docs/development.md) for
prerequisites, builds and checks. With prerequisites and
[Go Task](https://taskfile.dev/docs/installation) installed, use `task dev` to build
and run the app, or `task verify` for the standard checks. Fictional UI scenarios
and the disposable [backend lab](docs/backend-lab.md) support development without
production accounts.

## Documentation

- [Documentation index](docs/README.md), [changelog](docs/changelog.md) and [known limitations](docs/limitations.md)
- [Product requirements](docs/product/README.md) and [compatibility contract](docs/compatibility/README.md)
- [Architecture](docs/architecture/README.md) and [design reference](docs/design/README.md)
- [CI policy](docs/ci.md), [release tooling](tools/release/README.md) and [security reporting](SECURITY.md)

Licensed under [EUPL-1.2](LICENSE); third-party assets retain their
[notices and licenses](NOTICE.md). This is an independent project, not an official
NextCloud client.
