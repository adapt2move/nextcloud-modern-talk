# Security policy

This repository contains specifications and bootstrap work. No supported product
release or completed security audit is established here. Consult
[current state](docs/status.md) for implemented behavior and limitations;
[the security requirements](docs/quality/security.md) describe target contracts,
not delivered guarantees. A coarse security review is not a penetration test or
certification.

## Reporting privately

**Private reporting status: not yet verified.** This policy was prepared before
repository publication. No monitored private mailbox or response SLA has been
established by this work.

If available, use **Security → Report a vulnerability** on the
[repository security page](https://github.com/adapt2move/nextcloud-modern-talk/security).
Check that GitHub offers a private report before entering details. The link alone
does not establish that reporting is enabled or monitored.

If the private route is unavailable, an interim, **unverified contact approach**
is a content-free public issue asking the maintainers to establish a private
security channel. For example: “Please provide a private security contact.”
Do not include the vulnerability, exploit details, credentials, production data,
logs or attachments in that issue. Wait for a maintainer to establish the private
route before sharing sensitive information; this document does not promise a
response or identify an alternative verified address.

Once a private route is confirmed, provide the affected commit/version, platform,
impact, relevant requirement/module IDs and a minimal synthetic reproduction.
Use disposable accounts and redact tokens, real conversations and private host
details even in private reports. Maintainers do not need production credentials
to receive a report.

## Release preparation

Before a product release, maintainers must enable and verify GitHub private
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
