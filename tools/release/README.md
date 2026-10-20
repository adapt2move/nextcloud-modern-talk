# Release tooling

Build from a clean committed checkout using the repository toolchain and lockfile. Choose the matching native host; Windows is compile-only for this alpha.

```sh
python3 tools/release/toolchain.py --target x86_64-unknown-linux-gnu
python3 tools/release/build.py --target x86_64-unknown-linux-gnu \
  --version 0.0.3 --output /absolute/new/package-directory
```

Use `aarch64-apple-darwin` on Apple Silicon. Prepare native prerequisites as described in [CI](../../docs/ci.md). Output belongs outside the checkout. The builder records source/toolchain/dependency identities and collects license notices. The assembler checks the closed file inventory, executable architecture and checksums, then writes Linux `.deb`/`.tar.gz` or a self-contained macOS app ZIP and styled DMG, plus source and provenance.

When a published crate omits its license files, the builder can use the original upstream texts or explicitly identified official license texts in [notices/manifest.json](notices/manifest.json). Each supplement records its source and file hashes and must match the exact crate name, version, registry checksum and declared license. Collection verifies those checked-in files without fetching replacements; unresolved notices still stop packaging. Notice collection does not establish license approval.

The macOS builder embeds the required libraries, plugins and scanner from the checksum-pinned official GStreamer runtime. Native dependencies must resolve within the app or to macOS system libraries; external SDK, Homebrew and developer paths are rejected. It signs bundled code from the inside out with ad-hoc signatures and verifies the final bundle. There is no Developer ID signature or notarization. Retain project, font, audio, Lucide, Rust dependency and native runtime notices/source information when redistributing.

## Local tooling checks

```sh
python3 tools/release/check_workflows.py --actionlint
python3 tools/release/tests.py
python3 tools/release/publication_tests.py
```

Python 3.11+ and `dpkg-deb` are needed for the Linux packaging fixtures. These tests use synthetic executables and fake publication commands; they do not contact GitHub or qualify application behavior.

## Publication

The [manual workflow](../../.github/workflows/release.yml) binds an existing version tag to its exact mainline commit. Its publisher uses the official `gh` CLI. Packages are verified before a draft is created, then uploaded and downloaded to verify the served bytes before publication. Failures leave a draft; inspect it before a manual retry. Do not overwrite an existing release or move a published tag.

[actions.json](actions.json) contains action pins. [package-policy.json](package-policy.json) records the composition included by the package. Keep this policy aligned with the actual shipping source; it is not a substitute for runtime tests. [candidate.py](candidate.py) checks local tag/commit/origin identity. Evidence and intermediate reports stay outside contributor documentation.

## In-app updates

The application reads this repository's GitHub Releases for the user's selected stable or prerelease channel. Keep the package names and complete bundle layout consistent: the updater selects the exact platform asset and requires its GitHub SHA-256 digest. Source archives, checksums and provenance are release attachments, not executable update payloads.

Automatic checks run once per persisted randomized daily slot; Settings also offers an explicit check. A downloaded package becomes ready only after integrity and installation-route checks. The user initiates installation through the Update action, after calls and unsaved composition have been accounted for. Installation uses the verified staged package without another download and replaces the complete bundle. System-installed Linux packages use PackageKit; portable packages and writable macOS bundles use their supported replacement paths. Windows installation remains unavailable.

The initial v0.0.1 alpha has no in-app installer: users upgrade it manually. The macOS v0.0.2 updater predates the bundled-runtime layout and also requires one manual DMG upgrade to v0.0.3. Subsequent macOS updates require the complete self-contained runtime inventory. An ad-hoc signature does not establish a verified publisher identity or notarization. Do not move published tags or replace existing release assets to distribute an update.
