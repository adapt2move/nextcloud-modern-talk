# Development

Read [current state](status.md) and [Contributing](../CONTRIBUTING.md). Use a coherent checkout with its pinned dependencies; private historical snapshots are not contributor build inputs.

## Environment and toolchain

Contributors need Git, Python 3.11+, rustup and native platform prerequisites. Clone the [public repository](https://github.com/adapt2move/nextcloud-modern-talk), select the maintainer-designated revision and run commands from its root. No proprietary development service or production account is required.

For Ubuntu 24.04 x86_64 native builds and Xvfb rendering:

```sh
sudo bash tools/ui/prepare-linux.sh
sudo apt-get install -y --no-install-recommends libdbus-1-dev libglib2.0-dev dpkg-dev
sudo apt-get install -y --no-install-recommends xdg-utils desktop-file-utils xdg-desktop-portal xdg-desktop-portal-gtk
```

The script supplies compiler, pkg-config/Clang, X11/XCB/xkbcommon, font, SSL, Vulkan/Mesa, Xvfb, D-Bus and capture prerequisites. `libdbus-1-dev` supports the Linux credential provider; actual Secret Service tests additionally need a session bus and service. Bundled SQLite needs a C compiler. Do not change dependency features to bypass prerequisites.

The pinned Linux WebRTC SDK requires Clang 21 or later. On Ubuntu 24.04 the
script installs Clang 21 from [LLVM's signed package repository](https://apt.llvm.org/)
and verifies its signing-key fingerprint; use the explicit compiler names below.

Linux browser login uses the session's `org.freedesktop.portal.OpenURI` service and a registered default HTTPS browser. Minimal desktop sessions need a portal backend (GTK in the command above), `xdg-mime` from `xdg-utils`, and `update-desktop-database` from `desktop-file-utils` when registering a browser's desktop entry. Xvfb alone does not supply this browser handoff.

Linux screen-sharing capability checks use ashpd over the session bus, without the `gdbus` command or additional native build prerequisites. Actual ScreenCast availability requires a portal backend matching the desktop/compositor; installing the GTK backend for browser login does not establish ScreenCast support. Checks never open a source picker or capture session. Private portal fixtures require `dbus-run-session`, `dbus-daemon` and GNU `timeout` (coreutils); they use isolated buses with service activation disabled.

On Apple Silicon macOS, install the official GStreamer runtime and development framework as recorded in [prepare_gstreamer.py](../tools/ci/prepare_gstreamer.py), full Xcode and its macOS/Metal toolchain, select its developer directory, and verify `xcrun --sdk macosx --find metal` and `metallib`. Command Line Tools alone may be insufficient. No macOS runtime qualification is established.

For a local macOS Cargo build, set `RUSTFLAGS="-C link-arg=-Wl,-rpath,/Library/Frameworks/GStreamer.framework/Versions/1.0/lib"`. The official framework's pkg-config files supply link-time paths; this additional path lets the built app find its runtime libraries after launch. CI and the package builder set it explicitly and packaging checks the resolved library locations.

[rust-toolchain.toml](../rust-toolchain.toml) pins Rust 1.99.1, Clippy and rustfmt. Keep [Cargo.lock](../Cargo.lock) and use `--locked` for Cargo build/check/test/run commands. Coordinate dependency changes with the affected module owners. The toolchain helper reads the pin:

```sh
python3 tools/release/toolchain.py
rustc --version --verbose
cargo --version
```

The maintained `libwebrtc` binding needs its matching native SDK. Download and
verify the pinned artifact before building; the helper prints the directory to
use for `LK_CUSTOM_WEBRTC`:

```sh
export LK_CUSTOM_WEBRTC="$(python3 tools/media/prepare_native.py \
  --target linux-x64-release --output "$HOME/.cache/nextcloud-modern-talk/webrtc")"
export CC=clang-21 CXX=clang++-21
```

Use `mac-arm64-release` on Apple Silicon or `win-x64-release` for Windows MSVC.
The helper checks the archive digest and extracted inventory; do not substitute
an unrelated WebRTC build. The engine is a local library, not a LiveKit service.
Windows MSVC builds must set `RUSTFLAGS="-C target-feature=+crt-static"` and use
the explicit `--target x86_64-pc-windows-msvc`, matching the SDK's static runtime.

## Native UI entrypoint

For a fictional-data preview:

```sh
cargo run --locked -p talk-desktop -- --ui-scenario chat
```

This opens a native GPUI window without server login or capture. Launch without `--ui-scenario` for ordinary browser login and restored accounts. Previews, focus/keyboard checks and appearance fixtures do not establish integrated behavior. See [native UI boundaries](design/native-ui/README.md). Keep `--licenses` output and installed notices with packages.

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

In this agent workspace, application builds/tests/services use the author's assigned agent-dev-box session and frozen inputs. Respect the four-VM project limit, preserve other sessions and active inputs, sanitize evidence, remove disposable secret-bearing state and hibernate after use. A static documentation task needs no VM. This agent-specific workflow is not a prerequisite for ordinary OSS contributors.

User macOS testing is final-only after the assigned agent-side work and a prepared installable candidate. A compile/package result cannot establish Keychain, devices, permissions or VoiceOver behavior.

## Maintaining pins

[Action metadata](../tools/release/actions.json) records workflow pins; update commits only after checking official upstream source and runtime requirements. Retain actual runner image/SDK versions. Dependency changes need their owner's coordination and affected verification. See GitHub's [workflow security guidance](https://docs.github.com/en/actions/security-for-github-actions/security-guides/security-hardening-for-github-actions).
