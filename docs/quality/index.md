# Quality requirements and evidence

These documents define quality requirements for the Rust/GPUI client. They distinguish implemented behavior, tested compatibility and measured performance. Consult [current state](../status.md) for the delivered scope; a requirement, screenshot or successful compile is not a performance or interoperability result.

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

Choose verification in proportion to the change. Exercise observable behavior, failures and resource lifetime, and use the review scope agreed for the task. Requirements containing “must” remain acceptance conditions; performance numbers are provisional targets until the specified workload has been measured.

Supported server targets are NC34.x/Talk24.x and NC35.x/Talk25.x. Capabilities, server configuration, conversation attributes, permissions and session state determine available actions. A version string alone is insufficient. Windows/Linux storage adapters are interface targets; their presence here does not assert delivered operating-system support.

Evidence names the tested source, artifact, environment, scope and limitations. A missing test is `not run`, never `pass`. Keep detailed runs and intermediate reports outside normal contributor documentation; maintain a concise, accurate support inventory and release limitations.
