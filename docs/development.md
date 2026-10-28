# Development

Read [known limitations](limitations.md) and [Contributing](../CONTRIBUTING.md). Use a coherent checkout with its pinned dependencies; private historical snapshots are not contributor build inputs.

## Environment and toolchain

Contributors need Git, Python 3.11+, rustup and native platform prerequisites. Clone the [public repository](https://github.com/adapt2move/nextcloud-modern-talk), select the maintainer-designated revision and run commands from its root. No proprietary development service or production account is required.

For Ubuntu 24.04 x86_64 native builds and Xvfb rendering:

```sh
sudo bash tools/ui/prepare-linux.sh
sudo apt-get install -y --no-install-recommends libdbus-1-dev libglib2.0-dev dpkg-dev
sudo apt-get install -y --no-install-recommends xdg-utils desktop-file-utils xdg-desktop-portal xdg-desktop-portal-gtk
```

The script supplies compiler, pkg-config/Clang, X11/XCB/xkbcommon, font, SSL, Vulkan/Mesa, Xvfb, D-Bus and capture prerequisites. `libdbus-1-dev` supports the Linux credential provider; actual Secret Service tests additionally need a session bus and service. Bundled SQLite needs a C compiler. Do not change dependency features to bypass prerequisites.

The script uses Ubuntu's compiler packages. GStreamer's base/good plugins
provide the Opus and VP8 codecs used by the Rust WebRTC engine.

Linux browser login uses the session's `org.freedesktop.portal.OpenURI` service and a registered default HTTPS browser. Minimal desktop sessions need a portal backend (GTK in the command above), `xdg-mime` from `xdg-utils`, and `update-desktop-database` from `desktop-file-utils` when registering a browser's desktop entry. Xvfb alone does not supply this browser handoff.

Linux screen-sharing capability checks use ashpd over the session bus, without the `gdbus` command or additional native build prerequisites. Actual ScreenCast availability requires a portal backend matching the desktop/compositor; installing the GTK backend for browser login does not establish ScreenCast support. Checks never open a source picker or capture session. Private portal fixtures require `dbus-run-session`, `dbus-daemon` and GNU `timeout` (coreutils); they use isolated buses with service activation disabled.

On Apple Silicon macOS, install full Xcode and its macOS/Metal toolchain, select
its developer directory, and verify `xcrun --sdk macosx --find metal` and
`metallib`. Command Line Tools alone may be insufficient. Local development can
use Homebrew's SDK with `brew install pkgconf gstreamer`. CI and distributable
packages use the official runtime and development framework recorded in
[prepare_gstreamer.py](../tools/ci/prepare_gstreamer.py). No macOS runtime
qualification is established.

The task commands below find the selected GStreamer SDK through pkg-config,
with framework and Homebrew fallbacks, and add its runtime library search path.
No shell exports are needed for the usual local setup. They preserve compiler
flags and canonicalize macOS temporary paths for identity/storage tests. Direct
Cargo use remains available through [tools/cargo.py](../tools/cargo.py), for
example `python3 tools/cargo.py build --locked -p talk-desktop`. Packaging has its
own pinned SDK and clean-source checks; a Homebrew development build does not
replace them. The SDK is a developer prerequisite, not an end-user installation
step.

Before the first `task dev` on macOS, run `task dev:setup` once. It creates a
local development certificate in the current user's default keychain, trusted
only for code signing, and keeps its configuration in a private local application
support directory. The imported private key is non-extractable and grants
standing access only to Apple's `codesign` tool. Temporary key files are removed.
No app credential ACL or TLS trust policy is changed.

`task dev` builds and signs the debug executable with that same certificate and
fixed development identifier before starting it. Approve the newly signed app's
keychain access with **Always Allow** once when macOS asks. Later builds reuse
the same designated requirement, so macOS can recognize the app after recompilation.
Plain `cargo run`, `task build:debug` and ad-hoc release packages do not acquire
this development signature. See [Apple's code identity explanation](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements).
The development certificate does not provide Developer ID distribution or notarization.

[rust-toolchain.toml](../rust-toolchain.toml) pins Rust 1.99.1, Clippy and rustfmt. Keep [Cargo.lock](../Cargo.lock) and use `--locked` for Cargo build/check/test/run commands. Coordinate dependency changes with the affected module owners. The toolchain helper reads the pin:

```sh
python3 tools/release/toolchain.py
rustc --version --verbose
cargo --version
```

The `webrtc` and `rtc` crates supply the WebRTC protocol implementation.
Cargo resolves their exact versions and checksums from the lockfile; there is
no separate libwebrtc SDK download. GStreamer remains a native runtime
dependency for camera capture and media codecs. Windows builds use the explicit
`--target x86_64-pc-windows-msvc`.

## Native UI entrypoint

Install [Go Task](https://taskfile.dev/docs/installation) v3.54 or newer. On macOS,
use `brew install go-task`; this is Go Task, not Taskwarrior. Cargo remains the
Rust build tool. [Taskfile.yml](../Taskfile.yml) supplies common commands, and
[tools/cargo.py](../tools/cargo.py) supplies the same native environment in CI
without requiring Task on the runners.

| Command | Result |
| --- | --- |
| `task dev` | Build the debug desktop and start ordinary account-connected mode |
| `task dev:setup` | Configure the persistent local macOS development signing identity once |
| `task dev -- --ui-scenario chat --ui-preserve-preferences` | Build and start a fictional chat preview without saving preferences |
| `task build:debug` | Build `target/debug/talk-desktop` without starting it |
| `task build` | Build the optimized desktop in `target/release` using Cargo's release profile |
| `task check` | Check the workspace and all targets |
| `task test` | Run workspace tests |
| `task lint` | Run workspace Clippy with warnings denied |
| `task fmt` / `task fmt:check` | Apply / check Rust formatting |
| `task docs` | Check documentation links |
| `task --list` | List available commands |

On Windows, executable names end in `.exe`. Supplying Cargo's `--target` changes
the output directory to `target/<triple>/<profile>`. App arguments follow
`task dev --`; additional Cargo arguments follow the other tasks' `--` separator.
Cargo controls incremental rebuilds, so Task does not maintain a second file
timestamp cache.

`task build` produces a native release binary. To create distributable packages
on a matching native host with the pinned SDK:

```sh
task package -- --target aarch64-apple-darwin \
  --version 0.0.8 --output /absolute/new/package-directory
```

This delegates to the existing
[release builder](../tools/release/README.md), which requires a clean committed
checkout and a new output directory outside it.

For a fictional-data preview:

```sh
task dev -- --ui-scenario chat --ui-preserve-preferences
```

This opens a native GPUI window without server login or capture. Launch without `--ui-scenario` for ordinary browser login and restored accounts. Previews, focus/keyboard checks and appearance fixtures do not establish integrated behavior. See [native UI boundaries](design/README.md). Keep `--licenses` output and installed notices with packages.

To refresh the README image from the compiled Linux app:

```sh
python3 tools/ui/capture.py --binary target/debug/talk-desktop \
  --output /tmp/talk-readme --scenarios chat --width 1280 --height 720
```

Inspect `chat-light-blue.png` before copying it to `docs/assets/chat.png`.
Keep the README caption accurate about the platform and synthetic data.

Use `login-server-denied`, `chat-server-denied`, `chat-server-unavailable` and `accounts-server-denied` to inspect connection-error copy with fictional data.

Use `chat-send-pending` for the fictional pending-send presentation, repeated Enter and aligned delivery indicators on grouped, edited and wrapped messages. It deliberately leaves the send unconfirmed and does not qualify transport admission.

Use `chat-busy` to inspect rejected automatic read receipts and explicit sends under fictional local backpressure. Automatic requests remain quiet; a rejected send keeps its draft and shows a scoped notice for at most eight seconds. This fixture performs no server request.

Use `chat-reactions`, `chat-reactions-pending` and `chat-reactions-denied` for direct reactions, palette keyboard handling, confirmed badge toggles and duplicate-admission guards. These scenarios use fictional counts and never qualify a server mutation.

Use `chat-ux` and `chat-ux-call` for the header, search, reply and tooltip fixtures. They use fictional conversations and do not authorize a server send or call. Short transitions respect the observed system Reduce Motion preference; an unknown preference suppresses decorative motion. On Linux, observation uses the desktop's GNOME or KDE settings. The emoji action uses the system character palette on macOS, the Windows emoji picker, or an available Linux desktop picker. Linux picker selection may require an explicit copy and paste.

Update development builds are not treated as installable packages. For installation checks use the private fixture in [tools/updates](../tools/updates/install_probe.py), which creates a disposable package and never overwrites a system installation. The PackageKit fixture uses a private D-Bus service, not the system package manager. Runtime checks and package-layout fixtures do not qualify native macOS or Windows installation.

<a id="bootstrap-checks"></a>

## Workspace checks

Run the relevant checks from the repository root:

```sh
python3 tools/check_architecture.py
python3 tests/contracts/run_boundary_fixtures.py --candidate "$PWD" --output /absolute/new/boundary-results.json
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked -p talk-adapters --features backend-test --lib
```

Select the smallest relevant package scope for a change. Negative boundary fixtures must fail for their intended diagnostic; see [contract fixtures](../tests/contracts/README.md). A compile or synthetic fixture result does not qualify authentication, native media, accessibility or performance.

[CI](ci.md) runs tests on Linux and compilation only on Windows/macOS. The [release tooling guide](../tools/release/README.md) describes standalone workflow/package checks and their limits.

## Documentation check

```sh
python3 tools/check_docs.py
```

The [checker](../tools/check_docs.py) uses only the Python standard library and performs no network access or content execution. It scans Markdown relative to the repository; `--root` selects an isolated documentation tree.

It checks local inline/image and reference-style links, root-relative paths, heading fragments including duplicate headings, HTML IDs/named anchors and source `#L1-L3` ranges. It supports URL-encoded paths, balanced destination parentheses and angle-bracket destinations. Directory fragments resolve through a README or HTML index. Fenced/indented code, inline code and HTML comments are excluded; external URLs are not fetched. Generated/dependency directories and upstream research snapshots are excluded.

This bounded checker is not a complete CommonMark renderer or browser. It does not validate CSS references, JavaScript anchors, rustdoc symbols, external availability or requirement completeness. Unsupported non-document fragments are counted as unchecked. When changing the checker, exercise valid and deliberately broken links/fragments in a temporary tree outside the checkout before checking repository docs.

<a id="manual-integration-and-native-verification"></a>

## Execution ownership

The [Docker backend lab](backend-lab.md) provides pinned disposable server families and synthetic accounts. Keep real credentials out of ordinary PR jobs; use separate assigned tests for browser grants, native storage, media, devices and performance.

Build and test locally, in an isolated container or in CI. Preserve other contributors' work and remove disposable services and secret-bearing fixtures after use.

User macOS testing is final-only after the assigned agent-side work and a prepared installable candidate. A compile/package result cannot establish Keychain, devices, permissions or VoiceOver behavior.

## Maintaining pins

[Action metadata](../tools/release/actions.json) records workflow pins; update commits only after checking official upstream source and runtime requirements. Retain actual runner image/SDK versions. Dependency changes need their owner's coordination and affected verification. See GitHub's [workflow security guidance](https://docs.github.com/en/actions/security-for-github-actions/security-guides/security-hardening-for-github-actions).
