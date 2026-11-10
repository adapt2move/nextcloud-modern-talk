# Release tooling

Build from a clean committed checkout using the repository toolchain and lockfile. Choose the matching native host; Windows is compile-only for this alpha.

```sh
python3 tools/release/toolchain.py --target x86_64-unknown-linux-gnu
python3 tools/release/build.py --target x86_64-unknown-linux-gnu \
  --version <version> --output /absolute/new/package-directory
```

Use `aarch64-apple-darwin` on Apple Silicon. Prepare native prerequisites as described in [CI](../../docs/ci.md). Output belongs outside the checkout. The builder records source/toolchain/dependency identities and collects license notices. The assembler checks the closed file inventory, executable architecture and checksums, then writes Linux `.deb`/`.tar.gz` or a self-contained macOS app ZIP and styled DMG, plus source and provenance.

When a published crate omits its license files, the builder can use the original upstream texts or explicitly identified official license texts in [notices/manifest.json](notices/manifest.json). Each supplement records its source and file hashes and must match the exact crate name, version, registry checksum and declared license. Collection verifies those checked-in files without fetching replacements; unresolved notices still stop packaging. Notice collection does not establish license approval.

The macOS builder embeds the required libraries, plugins and scanner from the checksum-pinned official GStreamer runtime. Native dependencies must resolve within the app or to macOS system libraries; external SDK, Homebrew and developer paths are rejected. It signs bundled code from the inside out and verifies the final bundle. Publishing requires a persistent internal alpha certificate or Developer ID Application identity configured in [CI](../../docs/ci.md). Internal signatures pin the certificate and bundle identifier; Developer ID signatures verify the Apple team and enable hardened runtime. Neither mode changes end-user trust settings. Non-publishing builds remain ad-hoc signed. Notarization is not implemented. The legacy `macos-arm64-unsigned` asset-name suffix is retained for existing updater compatibility; the signed package provenance records the actual signer. Retain project, font, audio, Lucide, Rust dependency and native runtime notices/source information when redistributing.

## Local tooling checks

```sh
python3 tools/release/check_workflows.py --actionlint
python3 tools/release/tests.py
python3 tools/release/publication_tests.py
python3 tools/release/conventional_tests.py
```

Python 3.11+ and `dpkg-deb` are needed for the Linux packaging fixtures. These tests use synthetic executables and fake publication commands; they do not contact GitHub or qualify application behavior.

## Publication

Run the [release workflow](../../.github/workflows/release.yml) on `main`; it takes no version or commit.

```sh
gh workflow run release.yml --ref main            # release
gh workflow run release.yml --ref main -f publish=false   # trial: build only
```

1. [plan.py](plan.py) reads the Conventional Commits since the newest `vX.Y.Z` tag, picks the next version and writes the release notes. It stops if the checks have not passed for the commit or if only `docs`, `ci`, `test`, `refactor`, `build`, `style` or `chore` commits are pending.
2. [bump.py](bump.py) writes the version into `Cargo.toml`/`Cargo.lock` and the section into [the changelog](../../docs/changelog.md); the workflow commits it as `chore(release): vX.Y.Z`.
3. Linux and macOS build that exact commit.
4. The publisher tags it, verifies the packages, creates a draft, uploads, downloads and compares the bytes, then publishes the prerelease with the generated notes.

A failure before publication can be fixed and the workflow run again: the version commit and tag are reused, never duplicated. A failed publication leaves a draft; inspect it before retrying. Do not overwrite an existing release or move a published tag. The official `gh` CLI performs every GitHub mutation.

The bump follows [Conventional Commits](https://www.conventionalcommits.org/): a `!` after the type or a `BREAKING CHANGE:` footer is a major release (minor below 1.0); `feat` is a minor release (a patch below 1.0, where every release so far is a preview); `fix`, `perf`, `revert` and any commit outside the format are patches. CI rejects new commits outside the format. The notes group commits as breaking changes, features, fixes, performance and other changes, followed by version-bound links to [installation](../../docs/install.md) and [known limitations](../../docs/limitations.md).

[actions.json](actions.json) contains action pins. [package-policy.json](package-policy.json) records the composition included by the package. Keep this policy aligned with the actual shipping source; it is not a substitute for runtime tests. [candidate.py](candidate.py) checks local commit/origin identity, and the tag once publication creates it. Evidence and intermediate reports stay outside contributor documentation.

## In-app updates

The application reads this repository's GitHub Releases for the user's selected stable or prerelease channel. Keep the package names and complete bundle layout consistent: the updater selects the exact platform asset and requires its GitHub SHA-256 digest. Source archives, checksums and provenance are release attachments, not executable update payloads.

Automatic checks run once per persisted randomized daily slot; Settings also offers an explicit check. A downloaded package becomes ready only after integrity and installation-route checks. The user initiates installation through the Update action, after calls and unsaved composition have been accounted for. Installation uses the verified staged package without another download and replaces the complete bundle. System-installed Linux packages use PackageKit; portable packages and writable macOS bundles use their supported replacement paths. Windows installation remains unavailable.

The initial v0.0.1 alpha has no in-app installer: users upgrade it manually. The macOS v0.0.2 updater predates the bundled-runtime layout and also requires one manual DMG upgrade to v0.0.3. Subsequent macOS updates require the complete self-contained runtime inventory. An ad-hoc signature does not establish a verified publisher identity or notarization. Do not move published tags or replace existing release assets to distribute an update.
