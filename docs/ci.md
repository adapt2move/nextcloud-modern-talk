# Build, test and release

Checks are local commands first. The [automatic workflow](../.github/workflows/ci.yml)
runs `python3 tools/check.py`, also available as `task verify`, in one Linux job
on pull requests and pushes to `main`. It checks documentation, architecture,
workflow policy, Rust formatting, workspace Clippy and workspace tests. A new
push cancels the obsolete run for that PR or branch.

| Check | Where | Work |
| --- | --- | --- |
| `task verify` | Local host; automatic CI on `ubuntu-26.04` | The same standard checks, with no UI capture or packaging |
| `task verify:full` | Local development host with the required fixture tools | Also run tooling/package regressions, actionlint, backend-feature checks, permission callback lifetimes and isolated architecture fixtures |
| [Native release checks](../.github/workflows/native.yml) | Manual `windows-2025` and `macos-26` jobs | Compile workspace/test targets; on macOS also check the Swift helper, disposable signing identity and app/DMG assembly |
| [Release](../.github/workflows/release.yml) | Manual Linux/macOS jobs | Build and verify the actual distributable packages, then optionally publish |

Run `task verify` before pushing and choose the additional suites relevant to a
change. `python3 tools/check.py --full` runs the extended suites without Task;
each underlying script remains independently executable. Full tooling checks
include Linux-specific fixtures and the pinned Linux x86_64 actionlint binary.
UI captures remain a separate [local command](development.md#native-ui-entrypoint).

Before tagging a release, run `gh workflow run native.yml --ref main` and require
both native jobs to pass for the exact candidate, alongside the standard checks.
Use it earlier when changing platform code or dependencies. These jobs are
compile/package checks; they never launch the app or its Rust tests. The macOS
signing fixture uses a disposable identity, no release credentials or permission
prompts, and restores its temporary keychain search-list change. No trust or
authorization rules are changed. Neither these checks nor synthetic Linux
captures establish native device, credential-store, permission or UI behavior.

## Prerequisites and pins

[rust-toolchain.toml](../rust-toolchain.toml), [Cargo.lock](../Cargo.lock) and [action pins](../tools/release/actions.json) identify the toolchain, Rust dependencies and Actions. Cargo runs with `--locked`. CI caches registry downloads, not compiled application outputs. Release builds use a fresh target directory.

Automatic CI installs only the [build prerequisites](../tools/ui/prepare-linux.sh)
with `--build-only`, including libdbus and GLib headers. OCR, Xvfb, desktop portals
and capture tools are reserved for local UI work and Linux release smoke checks.
Windows needs MSVC and the pinned aws-lc crate's prebuilt NASM objects. macOS needs
Xcode and its Metal Toolchain; [build_env.py](../tools/ci/build_env.py) installs the
optional Metal component if missing and records the actual host.

[prepare_gstreamer.py](../tools/ci/prepare_gstreamer.py) installs checksum-pinned official SDKs on macOS and Windows. The Rust `webrtc` engine comes from the pinned Cargo dependencies and uses GStreamer for codecs, with no separate libwebrtc SDK download. Third-party native code remains upstream's responsibility; application Rust forbids `unsafe`.

Desktop/workspace compilation uses [tools/cargo.py](../tools/cargo.py), the same
native-environment starter used by the local [Task commands](development.md#native-ui-entrypoint).
CI does not install Task. The starter adds the selected macOS SDK's runtime
search path while preserving `-D warnings`; it does not install SDKs or execute
tests implicitly. Native checks state package, target and compile-only scope
explicitly. Packaging retains the separate pinned-runtime builder.

## Releases

The manual [release workflow](../.github/workflows/release.yml) accepts an exact commit on `main`, an existing version tag and an optional publish flag. It builds Linux `.deb`/`.tar.gz` and an Apple Silicon app ZIP plus a styled DMG. Linux installs and launches its packaged binary; macOS compiles and inspects the bundle, library paths, signatures and disk-image structure. The manual native check also assembles the macOS packages so packaging failures can be fixed before a release tag is created.

The separate publisher uses `ubuntu-26.04` and is the only job with `contents: write`. It checks package receipts and the existing tag, creates a draft using GitHub CLI, uploads files, downloads and compares their bytes, rechecks the tag, then marks the release as a prerelease. An error leaves the draft for inspection. It never replaces existing release assets or creates/moves a tag. Maintainers must serialize publication of a version; tag checks do not prevent an administrator from changing GitHub state concurrently.

macOS packages include the required GStreamer runtime, plugins, notices and source information. Publication requires a persistent identity in the Actions secrets `MACOS_CERTIFICATE_P12_BASE64` and `MACOS_CERTIFICATE_PASSWORD`. Configure one signing mode with public repository variables:

- `MACOS_SIGNING_MODE=internal` and `MACOS_SIGNING_CERTIFICATE=<40-character SHA-1 fingerprint>` use one self-signed alpha certificate. The app requirement binds the exact certificate to `org.adapt2move.nextcloud-modern-talk`. This provides identity continuity, with the existing alpha runtime profile; it supplies neither an Apple Team ID nor Gatekeeper publisher trust.
- `MACOS_SIGNING_MODE=developer-id` (the default) and `MACOS_SIGNING_TEAM=<10-character Apple Team ID>` use an existing Developer ID Application certificate. Nested code and the app require the same Apple team; hardened runtime and secure timestamps are enabled.

The job imports the chosen identity into an isolated temporary keychain, verifies every signature and deletes that keychain even after failure. Internal P12 containers are rewrapped for Apple's legacy importer with a password-protected 3DES/SHA-1 envelope and 100,000 iterations. This preserves the certificate, private key and SHA-256 certificate signature; decrypted private-key PEM stays in memory/pipes. Developer ID containers are imported unchanged. It never installs trust, changes the default keychain or generates a replacement release identity. The temporary keychain is appended to the runner user's search list for identity discovery, and cleanup restores the exact original list before deleting that keychain. Missing or mismatched credentials stop publication. Non-publishing packages remain ad-hoc signed; notarization is not implemented.

Without an Apple certificate, a maintainer can create the internal alpha identity **once** with `python3 tools/release/internal_identity.py --directory <private-backup-directory> --install-repository adapt2move/nextcloud-modern-talk`. The directory must have an existing parent. Keep the encrypted P12 and its password in that owner-private backup for subsequent releases; do not commit or upload them as artifacts. Re-running reuses the identity and refuses to overwrite existing repository signing configuration. Never generate a new certificate per release. The native macOS CI contract imports a separate disposable fixture signer and verifies stable requirements across two app versions and rejection of the wrong signer; it does not exercise TCC or the user's keychain.

Structural package/signing checks do not establish native installation, device or permission behavior. Windows binaries are compile-checked but are not packaged by this alpha workflow.

## Maintaining workflows

```sh
python3 tools/release/check_workflows.py --actionlint
python3 tools/ci/workflow_policy_tests.py
python3 tools/release/tests.py
python3 tools/release/publication_tests.py
```

The small policy checker enforces selected runners, pinned Actions, token scope and Linux-only test placement in our workflow layout. Upstream actionlint validates YAML and expressions. Neither tool is a sandbox or a general proof of scripts' behavior. Keep build helpers readable and review changes as ordinary source; there is no second helper hash-manifest system.
