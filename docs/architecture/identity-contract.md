# Account and identity boundary

[`AccountConnection`](../../crates/talk-adapters/src/identity/connection.rs) is the desktop composition interface. It owns one [`IdentityRuntime`](../../crates/talk-adapters/src/identity/runtime.rs), which owns browser login, durable account eligibility, credential providers and authenticated protocol clients. GPUI views receive bounded values and typed results; they cannot access credentials, provider records or authenticated URLs.

## Composition interface

Start once with `AccountConnection::start(RuntimeConfig::platform())`. Startup has an explicit state, so an empty initial snapshot is not mistaken for successful account loading. The runtime uses a separate thread and Tokio executor for network, ledger and provider work.

- `try_submit(ConnectionRequestId, RuntimeCommand)` admits bounded work without blocking the UI. IDs increase monotonically per facade. Busy means no admission.
- `snapshot()` returns at most eight accounts, with distinct saved, active, restoring, storage-recovery and reauthentication states. Dormant accounts have no fabricated live `AccountScope`.
- `poll(ConnectionCursor)` delivers retained, correlated completions. Acknowledge only a returned cursor. Sixteen retained jobs, including unacknowledged completions, bound the facade; a slow consumer gets Busy rather than silent result loss.
- `cancel_request`, `cancel_attempt`, `logout` and `shutdown` preserve a priority local closure path. `account_cancellation` and `new_operation_id` require the exact currently admitted scope.
- `is_stopped()` is nonblocking. `join()` belongs to a background owner, never a GPUI callback.

The runtime command/result definitions are the API reference. Composition translates them into [`talk_core::ui`](../../crates/talk-core/src/ui.rs), owns window revisions and rejects stale account/attempt/operation results. A stored-account revision is not a UI snapshot revision.

## Browser login and durable sign-in

`Begin` validates an HTTPS installation origin and opens Login Flow v2 in the system browser. `ReopenBrowser` uses the same live attempt. `Poll` obtains the one-time app credential privately and qualifies the authoritative canonical user and capabilities before `Save` can activate it. The app never receives the user's primary password through its production constructor; browser SSO/2FA stays with the server/browser. Polling, rather than a custom callback URL carrying credentials, completes this flow.

Normal sign-in persists the account. Native credential storage is preferred. If it is unavailable or denied, the user may explicitly choose the durable **unencrypted** private-file fallback. No automatic fallback or migration occurs. The low-level runtime retains a session-only command for explicit callers; it is not the ordinary sign-in default or the product's persistence promise.

`Restore` reads only the committed backend/generation and validates the privately stored expected canonical identity before returning a new live scope. Login name and canonical ID are separate values; neither is inferred from the other. Account labels and stable navigation keys are bounded private metadata, not authentication authority.

Production uses normal system trust and installation-relative HTTPS validation. Test-only loopback fixtures remain behind the default-off `backend-test` feature. They do not replace ordinary browser, certificate or storage verification.

## Cancellation, storage and logout

Unconsumed observations retain cancellation ownership, including an authorized Poll reply already enqueued for the UI. A cancelled save cannot activate through a late provider completion. Exact creator/attempt/generation correlation keeps an old observation from retiring a fresh attempt or another account.

The ledger separately tracks durable eligibility, restrictive decisions, writer settlement and cleanup. Local closure is immediate; durable suppression, remote leave and provider deletion are distinct acknowledgements. A failed suppression commit remains visible and owned for an explicit retry. Do not label an unavailable store or uncertain writer as successfully cleaned.

Removal follows `RetireAll` → `Maintain` → `ForgetAccount`, with exact-item cleanup and honest pending states. `Maintain` cannot grant restoration eligibility. Local logout suppression does not claim server-side app-password revocation or deletion of an offline content cache.

Private metadata uses bounded SQLite storage; credentials remain in the chosen provider. Private-file credentials use owner-only access controls and atomic generation files. The additive metadata migration preserves eligible credentials and stable account/navigation keys. Provider details and nonobvious transaction invariants stay beside the [ledger](../../crates/talk-adapters/src/identity/ledger.rs) and provider code.

Only the [authoritative rejection classifier](nextcloud-contract.md#i2-auth-rejection-classifier-01--authoritative-rejection) can confirm account loss. Ordinary room denials, malformed probes and transient failures do not revoke an account. Confirmed loss closes the shared account scope before a later completion can re-enable it.

## Limits and verification

The facade, runtime queues, provider workers and HTTP runtime all have explicit finite capacities; their constants and admission behavior are documented in the owning code. No caller may create another runtime to evade capacity, recreate an account to erase uncertain writes, or treat process exit as proof of provider settlement.

Linux real-server/browser, private-file and isolated Secret Service checks cover their named paths. Native macOS/Windows providers, sleep/wake, all device combinations and complete performance qualification remain separate. See [current limits](../status.md), [account requirements](../product/accounts.md) and the [development guide](../development.md) for supported checks.
