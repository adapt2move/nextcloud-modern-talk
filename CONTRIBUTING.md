# Contributing

This is an EUPL-1.2 project in the architecture/bootstrap phase. The accepted
[product specification review](docs/reviews/product-spec-review-r2.md) covers the
specification at `adfb44a2aab2e29ba6574c9dacc09475c534bccf`; it does not approve a
working client. Full product implementation waits for a recorded `G.BOOT`
acceptance. Read [current state](docs/status.md), the
[architecture](docs/architecture/README.md), and [development guide](docs/development.md)
before taking a task.

## Scope a contribution

Start with the [product requirements](docs/product/README.md),
[feature inventory](docs/compatibility/feature-inventory.md), and
[compatibility scenarios](docs/compatibility/test-matrix.md). Name the relevant
requirement IDs, `F-*` features, `CT-*` scenarios and owning modules in your change.
Preserve IDs; changing a requirement requires an explicit specification change and
renewed review of affected conclusions. Do not promote a Planned feature because
its interface or stub compiles.

Follow the Deep Modules boundaries: put consequential policy, credential,
transport, storage and media decisions behind the owning interfaces. Native GPUI
views consume those interfaces. Keep the domain and permission rules in the
specifications authoritative; a design image is not an API or a completed flow.
Compatibility depends on current capabilities, configuration, room policy and
effective permissions, with evidence for each claimed server profile.

For concurrent work, agree on owned paths and interface changes before editing.
Do not overwrite another contributor's work. An unresolved cross-module contract
belongs in the handoff, with an owner, rather than in an undocumented workaround.

## Verify and hand off

Use the [bootstrap commands](docs/development.md#bootstrap-checks) and proportionate
tests for the behavior changed. Follow the
[quality contracts](docs/quality/index.md), especially
[Q.VER-03](docs/quality/verification.md#qver-03--exact-candidate-and-evidence-identity)
for candidate identity and
[Q.VER-07](docs/quality/verification.md#qver-07--current-state-documentation-and-handoff)
for IST/current-state documentation. Describe actual before/after behavior, tests
run and their outcomes, tests not run, and limitations. Update current state and
affected interfaces/feature evidence in the same candidate. Keep targets,
fixtures, server observations and native results distinct.

The project manager delegates implementation and coordinates acceptance. Every
bootstrap or product candidate needs separately attributable, independent
architecture/modules, function/compatibility, performance and coarse-security
reviews under [Q.VER-02](docs/quality/verification.md#qver-02--independent-reviews-and-decision-authority).
Author checks support those reviews; they do not replace them. Preserve prior
findings, responses and unresolved objections between rounds. The manager's
acceptance names the exact committed candidate, accepted scope and remaining gates;
passing CI alone does not pass `G.BOOT`, `G.PRODUCT` or `G.RELEASE`.

Use synthetic accounts and sanitized evidence. Keep credentials, private server
names, private filesystem paths and raw sensitive traces out of commits, issues
and CI logs. Report vulnerabilities through [SECURITY.md](SECURITY.md).

## Documentation and attribution

Write public documentation in English; preserve German UI examples where specified.
Use relative links to repository documents and stable requirement anchors. Cite
official, version-pinned sources for protocol and dependency claims, and state
what was actually verified. See the [documentation check](docs/development.md#documentation-check)
for supported link syntax and limits.

Contribute material you have the right to distribute under the project's
[EUPL-1.2 license](LICENSE). Preserve third-party licenses and provenance in
[NOTICE.md](NOTICE.md); separately licensed assets keep their original terms.
