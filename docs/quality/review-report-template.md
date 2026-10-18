# Independent review — TEMPLATE, not an approval

Replace every placeholder. Preserve `not run` for missing evidence. Use a separate report/conclusion for architecture/modules, function/compatibility, performance and coarse security; see [review requirements](verification.md#qver-02--independent-reviews-and-decision-authority).

## Identity and scope

- Gate: `<G.SPEC | G.BOOT | G.PRODUCT | G.RELEASE>`
- Discipline: `<architecture/modules | function/compatibility | performance | coarse security>`
- Review round/date; reviewer identity; implementation/document author: `<...>`
- Independence statement: `<work inspected; confirm no approval of own implementation>`
- Original brief/spec revision; prior reports and unresolved objections: `<links>`
- Candidate full commit SHA; comparison/base SHA; clean/dirty state: `<...>`
- Artifact SHA-256; build/toolchain/dependency-lock identity: `<... or not applicable with reason>`
- Feature/module/quality IDs in scope; exclusions: `<...>`
- Server/lab manifest, platform/hardware and fixture identities: `<links or not run>`
- Approved pre-run workload/profile manifest and actual-profile validation; lifetime/peak collector qualification/coverage: `<links or not run — outside this gate>`

## Evidence and findings

| Requirement / feature / module ID | Assertion and environment | Evidence, command/run ID and artifact hash | Result | Limitations |
| --- | --- | --- | --- | --- |
| `<existing ID>` | `<observable expectation>` | `<reproducible evidence>` | `<pass / fail / not run / not applicable>` | `<...>` |

For each finding record:

- Finding ID/severity: `<...>`
- Affected requirement/module/feature and exact location: `<...>`
- Reproduction/preconditions, expected result and actual result: `<...>`
- Impact and evidence: `<...>`
- Required correction, owner and acceptance condition: `<...>`
- Author response/remediation commit: `<...>`
- Reviewer recheck, final candidate and evidence: `<...>`
- State: `<open / fixed pending verification / verified fixed / explicitly deferred>`

## Discipline-specific assessment

Fill the relevant paragraph; retain others as `out of this report's scope`.

- Architecture/modules: `<hidden decisions, interface size, dependency direction, state/error/cancellation semantics, account/secret boundaries and resource ownership; credential staging/restoration/crash ordering, account-loss fan-out/deadline, policy sources/numeric freshness/suspension/budget, workload/collector contracts; implementation departures>`
- Function/compatibility: `<positive and denied cases on both real server profiles; capability/config/permission changes; direct accept versus voluntary prejoin; missing parity and actual native behavior>`
- Performance: `<target versus measured values; frozen workload/actual-profile validation, sample counts, percentiles; complete process lifetimes and qualified peak upper bound versus simultaneous/sample maximum; app/helpers/media/GPU accounting; CPU/energy/network including policy refresh; collector/profiler overhead; regressions and unmeasured costs>`
- Coarse security: `<threat paths inspected; actual uncertain-vault-write/fallback/cancel/crash/restoration and account-loss shutdown behavior; TLS, content/log redaction, attachment/link handling, capture/consent/E2EE, bounded policy freshness/control-loss suspension and dependency scope; what was not audited>`

For a bootstrap review, state explicitly: **This conclusion concerns specification/contracts/scaffolding only. Product security, media integration and performance remain unverified.**

## Conclusion

- Verdict: `<pass for stated scope / changes required / insufficient evidence>`
- Blocking findings; nonblocking exceptions and expiry/owner: `<...>`
- Prior finding disposition and unresolved objections: `<...>`
- Final reviewed commit; intervening diff inspected; earlier evidence carried forward and why: `<...>`
- IST/feature-matrix corrections required: `<links>`
- Reviewer confirmation/date: `<...>`

The project manager's acceptance is a separate decision. A review is not a release authorization, full security audit or evidence for omitted features/platforms.
