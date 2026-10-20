# Build, test and release

The [CI workflow](../.github/workflows/ci.yml) has three jobs. Pull requests, pushes to `main` and manual runs use the same checks.

| Job | Runner | Work |
| --- | --- | --- |
| Linux | `blacksmith-4vcpu-ubuntu-2404` | Docs/policy checks, package fixtures, architecture and formatting, Clippy, Rust tests, desktop build and three native fixture captures |
| Windows | `blacksmith-4vcpu-windows-2025` | Compile the workspace and test targets for MSVC; do not execute them |
| macOS | `blacksmith-6vcpu-macos-latest` | Compile the workspace, test targets and native Swift helper for Apple Silicon; do not execute them |

Tests run only on Linux. Windows/macOS compilation does not establish native device, credential-store, permission or UI behavior. Linux synthetic captures are smoke checks, not a substitute for real server/device scenarios. New pull-request pushes cancel obsolete runs; a running main-branch run is allowed to finish.

## Prerequisites and pins

[rust-toolchain.toml](../rust-toolchain.toml), [Cargo.lock](../Cargo.lock) and [action pins](../tools/release/actions.json) identify the toolchain, Rust dependencies and Actions. Cargo runs with `--locked`. CI caches registry downloads, not compiled application outputs. Release builds use a fresh target directory.

Linux installs [native prerequisites](../tools/ui/prepare-linux.sh), libdbus and GLib development headers. Windows needs MSVC and uses the pinned aws-lc crate's prebuilt NASM objects. macOS needs Xcode and its Metal Toolchain; [build_env.py](../tools/ci/build_env.py) installs the optional Metal component if missing and records the actual host. Neither platform runs the app as a build check.

[prepare_gstreamer.py](../tools/ci/prepare_gstreamer.py) installs checksum-pinned official SDKs on macOS and Windows. The Rust `webrtc` engine comes from the pinned Cargo dependencies and uses GStreamer for codecs, with no separate libwebrtc SDK download. Third-party native code remains upstream's responsibility; application Rust forbids `unsafe`.

## Releases

The manual [release workflow](../.github/workflows/release.yml) accepts an exact commit on `main`, an existing version tag and an optional publish flag. It builds Linux `.deb`/`.tar.gz` and an Apple Silicon `.app.zip`. Linux installs and launches its packaged binary; macOS only compiles and assembles the bundle.

The separate publisher uses `blacksmith-2vcpu-ubuntu-2404` and is the only job with `contents: write`. It checks package receipts and the existing tag, creates a draft using GitHub CLI, uploads files, downloads and compares their bytes, rechecks the tag, then marks the release as a prerelease. An error leaves the draft for inspection. It never replaces existing release assets or creates/moves a tag. Maintainers must serialize publication of a version; tag checks do not prevent an administrator from changing GitHub state concurrently.

macOS packages are not Developer ID signed or notarized. They require the official GStreamer runtime listed in their installation instructions. A successful compile is not a Mac installation test. Windows binaries are compile-checked but are not packaged by this alpha workflow.

## Maintaining workflows

```sh
python3 tools/release/check_workflows.py --actionlint
python3 tools/ci/workflow_policy_tests.py
python3 tools/release/tests.py
python3 tools/release/publication_tests.py
```

The small policy checker enforces selected runners, pinned Actions, token scope and Linux-only test placement in our workflow layout. Upstream actionlint validates YAML and expressions. Neither tool is a sandbox or a general proof of scripts' behavior. Keep build helpers readable and review changes as ordinary source; there is no second helper hash-manifest system.
