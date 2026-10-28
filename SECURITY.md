# Security policy

NextCloud Modern Talk is alpha software. Security fixes target the current `main`
branch and latest alpha; older alpha releases have no separate maintenance branch.
No completed security audit is claimed. See [known limitations](docs/limitations.md).

## Report a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/adapt2move/nextcloud-modern-talk/security/advisories/new).
Private reporting is enabled for this repository. Do not disclose exploitable
vulnerabilities in public issues. If the private form is unavailable, wait until a
private reporting route is available before sending sensitive details.

Include the affected version or commit, platform, impact and a minimal synthetic
reproduction. Remove tokens, production credentials, real conversations and private
server details. There is no guaranteed response time.

## Contributing security fixes

Preserve account isolation, credential cleanup, origin-bound transport and explicit
media consent. [Security requirements](docs/quality/security.md) describe these
contracts; passing tests alone does not establish a security guarantee. Coordinate
disclosure and release timing through the private report.
