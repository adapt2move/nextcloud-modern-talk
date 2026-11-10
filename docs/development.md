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
For workspace checks without UI captures, use
`sudo bash tools/ui/prepare-linux.sh --build-only`; it also supplies libdbus and
GLib headers while omitting the OCR, capture and desktop-portal tools.

Linux browser login uses the session's `org.freedesktop.portal.OpenURI` service and a registered default HTTPS browser. Minimal desktop sessions need a portal backend (GTK in the command above), `xdg-mime` from `xdg-utils`, and `update-desktop-database` from `desktop-file-utils` when registering a browser's desktop entry. Xvfb alone does not supply this browser handoff.

Linux screen-sharing capability checks use ashpd over the session bus, without the `gdbus` command or additional native build prerequisites. Actual ScreenCast availability requires a portal backend matching the desktop/compositor; installing the GTK backend for browser login does not establish ScreenCast support. Permission checks never open a source picker or capture session. In-call sharing
uses that portal session and requires the `gstreamer1.0-pipewire` runtime plugin. Private portal fixtures require `dbus-run-session`, `dbus-daemon` and GNU `timeout` (coreutils); they use isolated buses with service activation disabled.

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
`--target x86_64-pc-windows-msvc`. The Cargo starter waits for the child process
on Windows and forwards its exit status; POSIX hosts replace the starter process.

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
| `task verify` | Run the same documentation, architecture, formatting, Clippy and workspace tests as CI |
| `task verify:full` | Also run tooling, package, backend-feature and architecture regressions |
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
  --version <version> --output /absolute/new/package-directory
```

This delegates to the existing
[release builder](../tools/release/README.md), which requires a clean committed
checkout and a new output directory outside it.

For a fictional-data preview:

```sh
task dev -- --ui-scenario chat --ui-preserve-preferences
```

This opens a native GPUI window without server login or capture. Launch without `--ui-scenario` for ordinary browser login and restored accounts. Previews, focus/keyboard checks and appearance fixtures do not establish integrated behavior. See [native UI boundaries](design/README.md). Keep `--licenses` output and installed notices with packages.

To refresh the README images from the compiled Linux app:

```sh
python3 tools/ui/capture.py --binary target/debug/talk-desktop \
  --output /tmp/talk-readme --width 1280 --height 800 --locale de \
  --scenarios chat,chat:dark,sidebar-activity:light:sage,call-reactions
```

Inspect the captures, then copy `chat-light-blue.png` to `docs/assets/chat.png`,
`chat-dark-blue.png` to `chat-dark.png`, `sidebar-activity-light-sage.png` to
`activity.png` and `call-reactions-light-blue.png` to `call.png`.
Keep the README caption accurate about the platform and synthetic data.

Use `login-server-denied`, `chat-server-denied`, `chat-server-unavailable` and `accounts-server-denied` to inspect connection-error copy with fictional data.

Use `login-account-store-busy`, `login-account-store-device-changed` and
`login-account-store-error` for local profile-load failures. They have no login
destination and expose storage actions instead of a browser-login retry. These
fixtures never reopen or repair an actual profile.

Use `sidebar-rich` for a realistic room list with unread counts, `sidebar-activity` and
`sidebar-activity-mentions` for the Activity page, `sidebar-new-menu`, `sidebar-list-menu`
and `sidebar-archive` for the list menus, and `sidebar-load-failed`, `chat-access-denied`
and `chat-read-only` for scoped error states. They are fictional and contact no server.
Export `TALK_UI_MATERIAL_PREVIEW=1` when capturing on Linux so textures and glass render.

Use `connection-reconnecting`, `offline-with-queue`, `connection-details` and
`startup-from-cache` for fictional offline/restart presentation (the status pill and its
popover). The capture tool waits through the normal
five-second grace period for connection scenarios. These fixtures perform no server
request and do not establish cache restoration, replay or native performance.

Use `chat-message-tools` for fictional location, reminder and scheduled-message
rows; `chat-reminder-menu`, `chat-reminder-time` and `chat-schedule-time`
show the message menu and date/time choosers; `lobby-settings` shows the
moderator lobby timer. These scenarios do not contact a server.

Use `chat-send-pending` for the fictional pending-send presentation, repeated Enter and aligned delivery indicators on grouped, edited and wrapped messages. It deliberately leaves the send unconfirmed and does not qualify transport admission.

Use `chat-message-states` for fictional own-message outcomes: read and sent messages, a pending, a queued and an unclear send with their wording under the message, one failed send as a quiet row above the composer (retry marks it retrying, Löschen discards the record, neither claims delivery) and a pinned message. `chat-delivery-states` and `chat-delivery-sent` show the read or sent receipt at the bottom right of each own message. `chat-message-hover`, `thread-message-hover` and `thread-message-menu` are the timeline and thread-pane scenarios for the hover toolbar and the "…" menu, which the native checks drive with the pointer. Linux fixtures do not prime the system timezone, so grouped same-sender rows are not formed in them; grouping stays covered by unit tests. These fixtures contact no server and do not qualify delivery.

Use `chat-busy` to inspect rejected automatic read receipts and explicit sends under fictional local backpressure. Automatic requests remain quiet; a rejected send keeps its draft and shows a scoped notice for at most eight seconds. This fixture performs no server request.

Use `chat-reactions`, `chat-reactions-pending` and `chat-reactions-denied` for direct reactions, palette keyboard handling, confirmed badge toggles and duplicate-admission guards. These scenarios use fictional counts and never qualify a server mutation.

Use `chat-content`, `chat-content-loading` and `chat-content-failed` to inspect proportional thumbnail tiles, labelled loading/failure states and the opaque media dialog. Open a tile, resize the window and check that the complete image stays inside its stage above the footer. These fixtures do not fetch files or establish server latency, actual retry timing or native macOS PDF support.

Use `chat-upload` and `chat-upload-uncertain` for the immediate outgoing image row, inline progress and missing-confirmation actions. These are fictional projections with no file/server effects.

Use `chat-attachments` for compact JSON/ZIP cards and the selected-file strip.
Remove chips, hover/focus Download and drop multiple local files into the chat.
The fixture shows a pending send rather than fabricating upload success. It has
no file or server I/O. Native Finder promises require a real macOS account-connected
run; an isolated Cocoa binding check or Linux UI run does not qualify them.

Use `chat-presence` to inspect fictional direct-message presence and custom status
in the sidebar and header. `status-picker` opens a fictional account-status
menu, and `status-message` opens its fictional status-message editor. These
fixtures do not query a server.

Use `call-gallery`, `call-speaker`, `call-screen-stage`, `call-dock`,
`call-participants-raised` and `call-recording` for fictional call presentation.
These fixtures do not connect screen streams or prove signaling, media, operating
system integration or accessibility. The screen stage keeps camera and screen
identities separate; the production media owner supplies scoped screen frames through the same stream metadata.
Use `search-navigation` for the fictional quick switcher, `account-search` for grouped message results and filters, `discover` for open-conversation browsing, and `search-filter` for the sidebar filters and muted-room indicator. These previews do not query a directory or establish server behavior.

Use `chat-ux` and `chat-ux-call` for the header, search, reply and tooltip fixtures. They use fictional conversations and do not authorize a server send or call. Short transitions respect the observed system Reduce Motion preference; an unknown preference suppresses decorative motion. On Linux, observation uses the desktop's GNOME or KDE settings. The emoji action uses the system character palette on macOS, the Windows emoji picker, or an available Linux desktop picker. Linux picker selection may require an explicit copy and paste.

Update development builds are not treated as installable packages. For installation checks use the private fixture in [tools/updates](../tools/updates/install_probe.py), which creates a disposable package and never overwrites a system installation. The PackageKit fixture uses a private D-Bus service, not the system package manager. Runtime checks and package-layout fixtures do not qualify native macOS or Windows installation.

<a id="bootstrap-checks"></a>

## Workspace checks

Run the relevant checks from the repository root:

```sh
task verify
# Extended tooling/contract suite on a prepared Linux development host:
task verify:full
```

Without Task, use `python3 tools/check.py` or `python3 tools/check.py --full`.
Both stop at the first failed command and return its failure status. No GitHub
runner variables or production accounts are needed. Individual checks remain
available for a smaller change:

```sh
python3 tools/check_architecture.py
python3 tests/contracts/run_boundary_fixtures.py --candidate "$PWD" --output /absolute/new/boundary-results.json
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked -p talk-adapters --features backend-test --lib
python3 -m unittest discover -s tools/identity -p 'test_*.py'
```

Select the smallest relevant package scope for a change. Negative boundary fixtures must fail for their intended diagnostic; see [contract fixtures](../tests/contracts/README.md). A compile or synthetic fixture result does not qualify authentication, native media, accessibility or performance.

[CI](ci.md) runs the standard check in one Linux job. Windows/macOS compilation
and macOS package validation are manual checks required before tagging a release.
The [release tooling guide](../tools/release/README.md) describes standalone
workflow/package checks and their limits.

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

The default-off `dev-probes` desktop feature provides a private lab profile and
bounded native latency/frame probes. Build the ordinary release profile with
assertions enabled (`RUSTFLAGS="-C debug-assertions=yes"`) and
`--features dev-probes`; this diagnostic build is not a distribution artifact.
`--dev-seed PROFILE FIXTURE` imports only an owner-private, literal-loopback
synthetic lab account, using the ordinary private-file account provider.
`--dev-profile PROFILE --dev-latency` opens it through normal composition and
records content after native frame callbacks. The feature never reads fixture
credentials into presentation or enables GPUI test support.

`tests/lab/native_latency.py --state PRIVATE_LAB_STATE --binary BUILT_BINARY
--output NEW_EVIDENCE_DIRECTORY` owns an isolated Xvfb display, two synthetic
rooms and three request-latency modes. `--recovery` adds a separate profile,
a disconnected relay and a 61-second SIGSTOP/SIGCONT interval; it checks catch-up
IDs and retirement of the queued send. `--performance --samples 1` instead seeds
200 rooms and over 1,000 mixed messages, verifies actual server counts, and runs
three 60-second native frame workloads. Preparation is outside timed intervals.
The runner preserves failed observations and private fixtures remain in `/run`
until cleanup. Keep native diagnostics separate from CPU-heavy build jobs.
See [performance](quality/performance.md) for boundaries and qualification limits.

## TURN over TLS check

The `media_flows_through_a_real_turn_server_over_tls` test is ignored by default because it needs a TURN server the test cannot start. The server must present the throwaway certificate in `crates/talk-adapters/src/media/fixtures/turn-test-cert.der` (CN `nmt-turn-test`, SAN `localhost` and `127.0.0.1`; its key sits beside it and is test-only). With Docker, convert both files to PEM with `openssl`, mount them and start coturn on the TLS port only:

```sh
docker run -d --name nmt-coturn -p 5349:5349 -v "$PEM_DIR:/certs:ro" coturn/coturn:latest \
  -n --lt-cred-mech --user nmt:secret --realm nmt.test --cert /certs/cert.pem \
  --pkey /certs/key.pem --no-udp --no-tcp --tls-listening-port 5349 \
  --min-port 49160 --max-port 49200 --allow-loopback-peers
NMT_TURNS_URL='turns:127.0.0.1:5349?transport=tcp' NMT_TURN_USER=nmt NMT_TURN_PASS=secret \
  python3 tools/cargo.py test --locked -p talk-adapters --lib -- media_flows_through_a_real_turn_server --ignored
```

Local ICE across many interfaces is not always reliable on a workstation; the test retries with fresh peers.
