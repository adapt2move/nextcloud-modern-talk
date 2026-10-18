# Testing and disposable integration fixtures

This is the manual integration entry for the bootstrap repository. The application remains a specification/module bootstrap; **all 140 product features remain Planned**. Backend HTTP probes do not demonstrate native functionality.

On 2026-10-18, the bounded backend smoke passed on NC34.0.5/Talk24.0.6 and NC35.0.2/Talk25.0.6. This author-executed evidence awaits independent review and commit binding; its scope and corrected failed attempts are recorded below.

- [Fixture setup, exact scenarios, reprovisioning and cleanup](../../tests/lab/README.md)
- [Immutable server/package pins](../../tests/lab/pins.json)
- [Run script](../../tests/lab/reprovision.sh) and [interrupted-run cleanup script](../../tests/lab/cleanup.sh)
- [Execution evidence and limitations](backend-smoke-2026-10-18.md)
- [Planned compatibility acceptance matrix](../compatibility/test-matrix.md), [verification gates](../quality/verification.md), and [lab contract](../quality/test-servers.md)

Run integration fixtures only in a disposable agent-dev-box environment with synthetic data. The fixture generates its own ephemeral credentials; no production account, credential environment variables or pull-request CI secrets are required. Preserve the active session's lifecycle ownership, inspect failed runs, remove disposable services/data, retrieve only sanitized evidence, and confirm hibernation when finished.

The small fixture checks pinned installation identities, Basic-auth API discovery, preset differences, private-room/chat isolation, poll closure and a selected channel restriction. The per-run evidence gives the exact completed operations and failures. These are partial backend observations within CT-001/002/004–007/010, not completion of those acceptance scenarios.

Browser Login Flow v2, 2FA/SSO, app-password issuance, OS Keychain/fallback, actual GPUI UI, direct-call/prejoin behavior, OS capture permissions, WebRTC/media, E2EE, HPB/TURN, accessibility and native performance require their own later runs. No server-only result can qualify them. General workspace contract tests and independent bootstrap review are also separate from this lab evidence.
