# Quality requirements and evidence

Status: requirements and research, 2026-10-18. These documents define acceptance for the intended Rust/GPUI client. They do not establish that authentication, native rendering, Talk integration, media, security controls or performance budgets have been implemented or verified. HTML prototypes and screenshots are design evidence only.

| Document | Purpose | Requirement IDs |
| --- | --- | --- |
| [Security](security.md) | Threat model, credential contract, privacy and media safeguards | `Q.SEC-01`–`Q.SEC-12` |
| [Performance](performance.md) | Provisional budgets, workloads and measurement rules | `Q.PERF-01`–`Q.PERF-08` |
| [Verification](verification.md) | Bootstrap and product gates, independent reviews and parity evidence | `Q.VER-01`–`Q.VER-08` |
| [Disposable test servers](test-servers.md) | Version research and the authorized future integration-test plan | `Q.LAB-01`–`Q.LAB-06` |
| [Review report template](review-report-template.md) | One report per independent review discipline | Template |
| [Acceptance report template](acceptance-report-template.md) | Project-manager decision after evidence and review | Template |
| [Workload manifest template](workload-manifest-template.md) | Inputs frozen before performance runs and actual-profile validation | Template for `Q.PERF-02/07`, `Q.LAB-05` |

[Product specifications](../product/README.md) own feature behavior and prioritization. Architecture documents own the Deep Modules design and concrete interface definitions. This directory adds cross-cutting obligations and observable acceptance conditions; it does not create an alternative feature catalog. Map its IDs to the existing product and module IDs in every report. IDs remain stable when wording changes; retire rather than reuse an ID.

The sequence is product specifications → architecture/module/interface bootstrap → delegated implementation → independent review → project-manager acceptance. A bootstrap pass permits implementation to begin. It is not a product release, media interoperability result or security certification. Requirements containing “must” are acceptance conditions; performance numbers are provisional targets pending measurement and review.

Supported server targets are NC34.x/Talk24.x and NC35.x/Talk25.x. Capabilities, server configuration, conversation attributes, permissions and session state determine available actions. A version string alone is insufficient. Windows/Linux storage adapters are interface targets; their presence here does not assert delivered operating-system support.

Evidence must name the exact commit, tested artifact, environment, scope and limitations. A missing test is `not run`, never `pass`. Current-state documentation (IST) must distinguish `specified`, `bootstrap`, `implemented, unverified`, `verified` and `deferred` with links to evidence. No measurements or live-server test results were produced during this authoring phase.
