# Security policy

This repository contains specifications and bootstrap work. No supported product
release or completed security audit is established here. Consult
[current state](docs/status.md) for implemented behavior and limitations;
[the security requirements](docs/quality/security.md) describe target contracts,
not delivered guarantees. A coarse security review is not a penetration test or
certification.

## Reporting privately

**Private reporting status: enabled and verified on 2026-10-18.** During
repository setup, GitHub's private-vulnerability-reporting API confirmed
`enabled: true` for `adapt2move/nextcloud-modern-talk` after enablement.
This confirms the repository setting; no monitoring arrangement, response SLA
or promise of a human response is established by this work.

Use **Security → Report a vulnerability** on the
[repository security page](https://github.com/adapt2move/nextcloud-modern-talk/security).
Confirm that GitHub offers a private report before entering details. If the route
becomes unavailable, do not post vulnerability details in a public issue; no
alternative private address is verified here.

In a private report, provide the affected commit/version, platform,
impact, relevant requirement/module IDs and a minimal synthetic reproduction.
Use disposable accounts and redact tokens, real conversations and private host
details even in private reports. Maintainers do not need production credentials
to receive a report.

## Release preparation

Before a product release, maintainers must reverify GitHub private
vulnerability reporting or another private route, confirm who monitors it, and
update this file with the verified route, supported versions and response
expectations. GitHub documents how to
[enable private vulnerability reporting](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/configure-for-a-repository).
Treat a route/configuration change as a reason to verify it again.

Release acceptance also requires the evidence in
[Q.SEC-11/12](docs/quality/security.md#qsec-11--dependencies-and-release-integrity)
and [G.RELEASE](docs/quality/verification.md#qver-01--ordered-gates). Publishing
source, a passing dependency check or a successful bootstrap build does not
establish credential safety, media privacy or a vulnerability-free application.
