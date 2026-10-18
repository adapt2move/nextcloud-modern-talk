# Development

Read [current state](status.md), the [architecture](architecture/README.md) and
[contributor process](../CONTRIBUTING.md) first. The bootstrap provides contracts
and maintenance infrastructure. The manager's [exact-source `G.BOOT` acceptance](reviews/bootstrap-acceptance.md)
permits a later delegated product increment; this documentation publication starts
none. The [ordered gates](quality/verification.md#qver-01--ordered-gates) continue to
govern changes and public claims.

## Environment and toolchain

Run commands from the repository root. Use Python 3.11 or later for the maintenance
scripts and rustup with the exact toolchain in
[rust-toolchain.toml](../rust-toolchain.toml). Keep [Cargo.lock](../Cargo.lock)
committed and use `--locked`; never fix CI by silently selecting a moving toolchain
or regenerating the lockfile during verification. Rustup's
[toolchain-file rules](https://rust-lang.github.io/rustup/overrides.html#the-toolchain-file)
explain selection and overrides; record the actual `rustc --version --verbose`
and `cargo --version` used.

In agent-dev-box sessions, source editing stays in the shared workspace. The
designated lab operator owns synchronization, provision/start/stop and cleanup;
authors do not create parallel VMs or run host builds/services/tests. The verifier
runs the commands below through the coordinated development VM after all owned
outputs are ready. Record the candidate, environment and commands, then let the
lab operator clean up and hibernate. An unavailable native Mac remains a native
verification limitation.

## Bootstrap checks

```sh
python3 tools/check_docs.py
python3 tools/check_architecture.py
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

These check documentation, declared architecture and the workspace's default
feature set, including Rust documentation tests. The architecture checker owns its
documented structural assertions; neither script proves the design is sufficient.
Add focused tests when behavior exists, including denied, stale, cancelled and
failure paths. Do not create passing placeholder tests for absent adapters.

The [CI workflow](../.github/workflows/ci.yml) runs this sequence on an ephemeral
`ubuntu-24.04` GitHub runner for pull requests, pushes to `main` and manual
`workflow_dispatch`. It has only `contents: read`, does not persist checkout
credentials, and requests no repository secrets, fixture services or publication.
Manual dispatch repeats bootstrap checks; it does not run server integration.
Linux results cover only the components and assertions actually exercised. They
cannot qualify macOS GPUI, Keychain, capture, accessibility, media interoperability
or the native [performance budgets](quality/performance.md).

Verification results belong in [current state](status.md) and the candidate's
review evidence. This guide defines commands; it records no executed pass.

## Documentation check

[tools/check_docs.py](../tools/check_docs.py) uses only the Python standard library.
It finds Markdown documents relative to its repository, independently of the
caller's working directory. `--root` selects an isolated documentation tree for
verification fixtures. It performs no network access or content execution.

The checker validates local inline/image and reference-style destinations,
repository-root paths beginning with `/`, Markdown heading fragments (including
duplicate headings), explicit HTML `id`/named anchors and source-file `#L1-L3` line
references. URL-encoded filenames/fragments, balanced destination parentheses and
angle-bracket destinations are supported. Directory links without a fragment are
valid; directory fragments use a README or HTML index. Raw HTML links/images in
Markdown are included. Prefer simple headings or explicit IDs for stable anchors.

Fenced/indented code, inline code and HTML comments are examples rather than link
sources. External URLs are not fetched. Generated/dependency directories and
historical upstream fetches under `design/research/` or `docs/design/research/`
are excluded from source scanning. Existing repository review reports remain in
scope. Paths escaping the selected root are errors.

This is a bounded Markdown check, not a complete CommonMark renderer or a browser.
It does not evaluate JavaScript-generated anchors, CSS asset references, rustdoc
symbol URLs, external availability, prose-only IDs or requirement completeness.
Unsupported fragment formats on non-Markdown/non-HTML files are counted as
unchecked in the summary. A successful exit is only evidence for the reported
checks. Planned/TODO prose is not a failure; an actual link to an absent target is.

When changing the checker, the designated verifier should use a small temporary
tree outside the source checkout. Include a passing relative link and fragment,
an encoded filename, a duplicate heading, an HTML ID and a source-line range.
Then independently introduce a missing file, missing fragment and undefined
explicit reference label; each run must exit nonzero and identify its source line.
Restore the valid links and add broken-looking examples inside a code fence and
inline code; they must not create failures. Run the repository check after this
negative fixture. Keep fixture output separate from application/integration tests.

## Manual integration and native verification

For an authorized integration run, a maintainer selects the exact reviewed
candidate and claimed requirement/scenario IDs, then coordinates a disposable VM
profile under the [lab contract](quality/test-servers.md) and
[testing guide](testing/README.md). Pin both Nextcloud/Talk patch pairs and record
actual capabilities/configuration. Use synthetic accounts, sanitize evidence and
destroy disposable data/services before hibernation. Real server observations,
fault fixtures and native client behavior have separate outcomes.

There is no automated integration workflow in this bootstrap. Before adding one,
review its fixture lifecycle, credential handling, fork-PR isolation and cleanup.
Do not connect arbitrary pull-request code to private servers, persistent runners
or secrets. Native Mac checks and qualified performance workloads remain explicit
later work under [Q.VER](quality/verification.md), [Q.SEC](quality/security.md) and
[Q.PERF](quality/performance.md); a healthy disposable server does not complete a
native feature.

## Maintaining pins

The workflow uses the official
[actions/checkout v7.0.1 release](https://github.com/actions/checkout/releases/tag/v7.0.1)
at [commit 3d3c42e5aac5ba805825da76410c181273ba90b1](https://github.com/actions/checkout/commit/3d3c42e5aac5ba805825da76410c181273ba90b1),
verified against the upstream release and
[pinned action documentation](https://github.com/actions/checkout/blob/3d3c42e5aac5ba805825da76410c181273ba90b1/README.md)
on 2026-10-18. The runner image is an OS label, not an immutable image; preserve
the actual runner/tool versions with evidence. Action and Rust pins improve
repeatability without establishing bit-for-bit build reproducibility.

The initial Rust pin is `1.99.1`, verified against the
[official release announcement](https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/)
on 2026-10-18. This verifies release existence; installation and workspace checks
still require execution on the selected verification environment.

For updates, verify the proposed action commit belongs to the official upstream,
review its release/runtime requirements, and retain a full commit SHA in the
workflow. Check Rust changes against official release evidence, update the
toolchain and lockfile deliberately, and rerun affected checks/reviews. GitHub's
[workflow security guidance](https://docs.github.com/en/actions/security-for-github-actions/security-guides/security-hardening-for-github-actions)
and [permission reference](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#permissions)
explain the minimal-token and untrusted-code boundaries used here.
