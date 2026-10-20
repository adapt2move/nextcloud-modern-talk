# Documentation

Read [current state](status.md) for limitations and [development](development.md) to contribute. Specifications describe intended behavior; they do not establish implemented support.

| Read | Purpose |
| --- | --- |
| [Development](development.md) | Prerequisites, pinned toolchain, native UI entrypoint and scoped checks |
| [Alpha notes](alpha.md) | Installation, included functionality and known limitations |
| [Backend lab](backend-lab.md) | Disposable pinned servers, synthetic accounts and cleanup |
| [Product](product/README.md) | Intended user outcomes and stable requirement IDs |
| [Compatibility](compatibility/README.md) | Server families, runtime conditions, inventory and scenarios |
| [Design](design/README.md) | Design hierarchy, flows, tokens and illustrative assets |
| [Architecture](architecture/README.md) | Module responsibilities, boundaries and data flow |
| [Quality](quality/index.md) | Security, performance and verification requirements |
| [CI](ci.md) | Linux tests, Windows/macOS compile-only jobs |
| [Release tooling](../tools/release/README.md) | Package inputs, metadata and publication commands |

<a id="active-implementation-and-credential-amendment"></a>

Normal sign-in persists the account, preferring a native credential store and offering an explicit durable unencrypted private-file fallback. The [account contract](product/accounts.md) describes storage failure, restoration and logout. Platform qualification remains separate from the storage interface.

Keep implementation rationale beside code. Historical reports and task ledgers belong outside normal contributor documentation; follow [AGENTS.md](../AGENTS.md).
