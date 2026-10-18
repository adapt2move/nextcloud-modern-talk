# Acceptance decision — TEMPLATE, not an approval

Complete only after the [required independent reviews](verification.md). An author-generated checklist cannot replace them.

## Candidate and proposed claim

- Gate, date and project manager: `<...>`
- Exact final commit SHA, clean/dirty state and artifact SHA-256: `<...>`
- Accepted product/spec revision; bootstrap decision if entering product implementation: `<links>`
- Delegated author/implementation handoff and scope: `<link>`
- Proposed accepted scope and user-visible claim: `<precise increment; no implied full app/parity>`
- Excluded/deferred/unverified features/platforms: `<...>`
- IST/current-state document and authoritative feature matrix, updated in candidate: `<links>`
- Bootstrap contract decisions: `<credential generations/restoration/crash ordering; account-loss shutdown/classifier/release deadline; policy owners/sources/numeric bounds/suspension/budget; workload schema and lifetime/peak collector plan>`

## Independent review record

| Discipline | Independent reviewer | Report and evidence | Final commit confirmed | Verdict / open findings |
| --- | --- | --- | --- | --- |
| Architecture/modules | `<...>` | `<...>` | `<SHA>` | `<...>` |
| Function/compatibility | `<...>` | `<...>` | `<SHA>` | `<...>` |
| Performance | `<...>` | `<...>` | `<SHA>` | `<...>` |
| Coarse security | `<...>` | `<...>` | `<SHA>` | `<...>` |

For each row, record independence from the author. Attach remediation/recheck evidence and carry-forward justification if earlier reviews apply to the final commit. Unresolved blocking findings prevent acceptance.

## Evidence scope

| Required area | Evidence and exact environment | Result / remaining gap |
| --- | --- | --- |
| Product IDs → modules → acceptance tests | `<matrix link>` | `<...>` |
| NC34.x/Talk24.x and NC35.x/Talk25.x | `<exact installed patches, pins, real-server runs>` | `<...>` |
| Capability/configuration/permission changes and failures | `<positive/negative scenarios>` | `<...>` |
| Native macOS UI, Keychain and session-only fallback | `<actual native artifact/device evidence; uncertain writes/cancel/crash/restoration and account-loss cases>` | `<...>` |
| Media, permissions, direct accept/prejoin, consent/E2EE | `<profile, engine and peer versions; observed media; account-loss shutdown and policy observation/expiry/suspension evidence>` | `<...>` |
| Performance budgets and regression scope | `<reference hardware; pre-run approved workload/profile hash and actual-profile validation; lifetime/peak qualification, coverage, raw runs and traces>` | `<...>` |
| Security/privacy and dependency scope | `<negative tests/review; explicit limitations>` | `<...>` |
| Lab cleanup, secret removal and VM state/cost | `<sanitized cleanup record>` | `<...>` |
| Public documentation and release claims | `<IST/design labels/redaction/disclosure/packaging as applicable>` | `<...>` |

For `G.SPEC`/`G.BOOT`, mark runtime rows `not run — outside this gate`; do not turn them into product passes. All product-dependent evidence remains required before the corresponding product claim.

## Decision and obligations

- Decision: `<accept stated gate and scope / reject / awaiting evidence>`
- Final accepted commit: `<SHA>`
- Blocking findings closed with independent recheck: `<links>`
- Permitted lower-impact exceptions: `<finding, rationale, owner, deadline/milestone, narrowed claim>`
- Remaining product/release gates and next delegated work: `<...>`
- Public wording approved: `<e.g. “Architecture bootstrap; native client features remain unimplemented/unverified.”>`
- Project-manager sign-off/date: `<...>`

A bootstrap decision permits implementation. It does not certify security, demonstrate media interoperability or establish measured performance. A later code/dependency change needs affected evidence and reviews renewed.
