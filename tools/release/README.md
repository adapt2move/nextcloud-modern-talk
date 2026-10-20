# Release tooling

Build from a clean committed checkout using the repository toolchain and lockfile. Choose the matching native host; Windows is compile-only for this alpha.

```sh
python3 tools/release/toolchain.py --target x86_64-unknown-linux-gnu
python3 tools/release/build.py --target x86_64-unknown-linux-gnu \
  --version 0.0.1 --output /absolute/new/package-directory
```

Use `aarch64-apple-darwin` on Apple Silicon. Prepare native prerequisites as described in [CI](../../docs/ci.md). Output belongs outside the checkout. The builder records source/toolchain/dependency identities and collects license notices. The assembler checks the closed file inventory, executable architecture and checksums, then writes Linux `.deb`/`.tar.gz` or an unsigned macOS app ZIP plus source and provenance.

When a published crate omits its license files, the builder can use the original upstream texts or explicitly identified official license texts in [notices/manifest.json](notices/manifest.json). Each supplement records its source and file hashes and must match the exact crate name, version, registry checksum and declared license. Collection verifies those checked-in files without fetching replacements; unresolved notices still stop packaging. Notice collection does not establish license approval.

The macOS alpha requires the official GStreamer runtime specified in the package instructions. It has no Developer ID signature or notarization. Do not describe compilation as native platform qualification. Retain project, font, audio, Rust dependency and native WebRTC notices when redistributing.

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
