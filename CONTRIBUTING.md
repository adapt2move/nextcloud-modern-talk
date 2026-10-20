# Contributing

NextCloud Modern Talk is an EUPL-1.2 Rust/GPUI project for teams. Read [current state](docs/status.md), [development](docs/development.md) and [architecture](docs/architecture/README.md) before taking a task. Coordinate owned paths when working concurrently.

## Scope a contribution

Use the [product requirements](docs/product/README.md), [feature inventory](docs/compatibility/feature-inventory.md) and [compatibility scenarios](docs/compatibility/test-matrix.md). Name relevant requirement, feature and scenario IDs. Preserve those IDs and make requirement changes explicit. An interface, stub or successful compile does not verify a feature.

Keep transport, credentials, persistence and media behind their owning modules. GPUI views consume bounded snapshots and typed intents. Specifications govern permissions and supported behavior; design images are illustrative. Compatibility claims need evidence for the actual server, configuration and platform.

For concurrent work, agree on owned paths and interface changes. Preserve other contributors' edits and active private inputs. Escalate cross-module contract changes to their owner.

## Verify a change

Choose proportionate checks using the [development guide](docs/development.md#bootstrap-checks) and [quality requirements](docs/quality/index.md). Describe changed behavior, source identity, checks run, outcomes, unrun checks and real limitations. Keep synthetic fixtures, server observations, native results and compilation distinct. Follow [Q.VER-03](docs/quality/verification.md#qver-03--exact-candidate-and-evidence-identity) for consequential verification and [Q.VER-07](docs/quality/verification.md#qver-07--current-state-documentation-and-handoff) for current-state changes.

The task owner selects review scope in proportion to the change. Product and release claims still require evidence and acceptance for their actual scope under [Q.VER-02](docs/quality/verification.md#qver-02--independent-reviews-and-decision-authority); passing CI alone establishes neither.

Use synthetic accounts and sanitized evidence. Keep credentials, private server names, private paths and raw sensitive traces out of commits, issues and CI logs. Report vulnerabilities through [SECURITY.md](SECURITY.md).

## Documentation and attribution

Write public documentation in English and use NextCloud for the project's brand. Preserve specified localized UI examples. Keep contributor instructions concise, product specifications normative, and high-level architecture focused on responsibilities and boundaries. Put implementation rationale beside code. Keep historical reports and task ledgers outside normal contributor documentation.

Use relative links and stable requirement anchors. Cite official, pinned sources for protocol and dependency claims. Run the [documentation check](docs/development.md#documentation-check) for changed links.

Preserve [EUPL-1.2](LICENSE) and third-party terms and provenance in [NOTICE.md](NOTICE.md). Contribute only material you have the right to distribute; separately licensed assets retain their terms.
