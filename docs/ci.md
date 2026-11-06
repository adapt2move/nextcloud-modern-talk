# Build, test and release

The [CI workflow](../.github/workflows/ci.yml) runs only the Linux job on pull requests.
Pushes to `main` and manual runs on `main` also run Windows and macOS. Manual runs
on other branches run Linux only by default. An explicit manual `signing_probe`
run checks only a disposable macOS signing identity on the selected ref. It does
not use release credentials or run the application, and does not replace full CI.
The probe installs no trust, changes no authorization rules and does not affect
end-user Macs. Its disposable keychain is temporarily included in the runner
user's search list; teardown restores the original list without changing the
default keychain.

| Job | Runner | Work |
| --- | --- | --- |
| Linux | `ubuntu-26.04` | Docs/policy checks, package fixtures, architecture and formatting, Clippy, Rust tests, desktop build and three native fixture captures |
| Windows | `windows-2025` | Compile the workspace and test targets for MSVC; do not execute them |
| macOS | `macos-26` | Compile the workspace, test targets and native Swift helper for Apple Silicon; assemble and inspect the self-contained app/DMG without executing the app |
| Manual signing probe | `macos-26` | With `signing_probe=true`, compile two inert C fixtures and check disposable signing identity continuity; no release credentials, app launch or permission prompts |

These standard GitHub-hosted runners are free for this public repository. macOS uses the Apple Silicon runner.

Rust/application tests run only on Linux. Windows/macOS compilation and the signing probe do not establish native device, credential-store, permission or UI behavior. Linux synthetic captures are smoke checks, not a substitute for real server/device scenarios. New pull-request pushes cancel obsolete runs; a running main-branch run is allowed to finish.

## Prerequisites and pins

[rust-toolchain.toml](../rust-toolchain.toml), [Cargo.lock](../Cargo.lock) and [action pins](../tools/release/actions.json) identify the toolchain, Rust dependencies and Actions. Cargo runs with `--locked`. CI caches registry downloads, not compiled application outputs. Release builds use a fresh target directory.

Linux installs [native prerequisites](../tools/ui/prepare-linux.sh), libdbus and GLib development headers. Windows needs MSVC and uses the pinned aws-lc crate's prebuilt NASM objects. macOS needs Xcode and its Metal Toolchain; [build_env.py](../tools/ci/build_env.py) installs the optional Metal component if missing and records the actual host. Neither platform runs the app as a build check.

[prepare_gstreamer.py](../tools/ci/prepare_gstreamer.py) installs checksum-pinned official SDKs on macOS and Windows. The Rust `webrtc` engine comes from the pinned Cargo dependencies and uses GStreamer for codecs, with no separate libwebrtc SDK download. Third-party native code remains upstream's responsibility; application Rust forbids `unsafe`.

Desktop/workspace compilation uses [tools/cargo.py](../tools/cargo.py), the same
native-environment starter used by the local [Task commands](development.md#native-ui-entrypoint).
CI does not install Task. The starter adds the selected macOS SDK's runtime
search path while preserving `-D warnings`; it does not install SDKs or execute
tests implicitly. The workflow still states package, target and compile-only
scope explicitly. Packaging retains the separate pinned-runtime builder.

## Releases

The manual [release workflow](../.github/workflows/release.yml) accepts an exact commit on `main`, an existing version tag and an optional publish flag. It builds Linux `.deb`/`.tar.gz` and an Apple Silicon app ZIP plus a styled DMG. Linux installs and launches its packaged binary; macOS compiles and inspects the bundle, library paths, signatures and disk-image structure. The ordinary macOS CI job also assembles these packages so packaging failures can be fixed before a release tag is created.

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
