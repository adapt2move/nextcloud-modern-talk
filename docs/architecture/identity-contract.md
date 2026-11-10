# Account and identity boundary

[`AccountConnection`](../../crates/talk-adapters/src/identity/connection.rs) is the desktop composition interface. It owns one [`IdentityRuntime`](../../crates/talk-adapters/src/identity/runtime.rs), which owns browser login, durable account eligibility, credential providers and authenticated protocol clients. GPUI views receive bounded values and typed results; they cannot access credentials, provider records or authenticated URLs.

## Composition interface

Start once with `AccountConnection::start(RuntimeConfig::platform())`. Startup has an explicit state, so an empty initial snapshot is not mistaken for successful account loading. The runtime uses a separate thread and Tokio executor for network, ledger and provider work.

- `try_submit(ConnectionRequestId, RuntimeCommand)` admits bounded work without blocking the UI. IDs increase monotonically per facade. Busy means no admission.
- `snapshot()` returns at most eight accounts, with distinct saved, active, restoring, storage-recovery and reauthentication states. Dormant accounts have no fabricated live `AccountScope`.
- `poll(ConnectionCursor)` delivers retained, correlated completions. Acknowledge only a returned cursor. Sixteen retained jobs, including unacknowledged completions, bound the facade; a slow consumer gets Busy rather than silent result loss.
- `cancel_request`, `cancel_attempt`, `logout` and `shutdown` preserve a priority local closure path. `account_cancellation` and `new_operation_id` require the exact currently admitted scope.
- `is_stopped()` is nonblocking. `join()` belongs to a background owner, never a GPUI callback.
- `retry_startup()` replaces only a failed, stopped initial owner with no accounts
  or retained jobs. It never restarts an owner that reached ready state or retries
  a credential operation. A profile-load failure has its own UI reason and is not
  presented as a failed browser login or credential save.

The runtime command/result definitions are the API reference. Composition translates them into [`talk_core::ui`](../../crates/talk-core/src/ui.rs), owns window revisions and rejects stale account/attempt/operation results. A stored-account revision is not a UI snapshot revision.

## Browser login and durable sign-in

`Begin` validates an HTTPS installation origin and opens Login Flow v2 in the system browser. `ReopenBrowser` uses the same live attempt. `Poll` obtains the one-time app credential privately and qualifies the authoritative canonical user and capabilities before `Save` can activate it. The app never receives the user's primary password through its production constructor; browser SSO/2FA stays with the server/browser. Polling, rather than a custom callback URL carrying credentials, completes this flow.

Normal sign-in persists the account. Native credential storage is preferred. If it is unavailable or denied, the user may explicitly choose the durable **unencrypted** private-file fallback. No automatic fallback or migration occurs. The low-level runtime retains a session-only command for explicit callers; it is not the ordinary sign-in default or the product's persistence promise.

`Restore` reads only the committed backend/generation and validates the privately stored expected canonical identity before returning a new live scope. Login name and canonical ID are separate values; neither is inferred from the other. Account labels and stable navigation keys are bounded private metadata, not authentication authority.

A credential read retains one provider operation for up to 60 seconds, allowing an OS permission sheet to complete without discarding its approved result. Cancellation is observed within 50 ms; an unfinished blocking read keeps its existing worker permit until it returns. Timeout is a temporary storage failure, not a server timeout or loss of saved authorization. Network validation starts after the read and keeps its separate deadline. Maintenance and uncertain-write settlement retain their existing three-second observation slices.

Linux credential operations use one encrypted `dbus-secret-service` session per
operation on the existing provider worker. Collection/item checks refuse locked
storage and never dispatch Unlock or Prompt. Typed D-Bus error identifiers keep an
absent bus distinct from missing credentials and access denial. The old zbus
keyring provider is a test-only compatibility fixture for existing records.
Credential access, ordinary content persistence, browser handoff and PackageKit
transactions retain their own owners and failure scopes. The asynchronous Linux
adapters use zbus 6 on their existing Tokio executor with explicit client features;
service-side features are enabled only for isolated tests.

Production uses normal system trust and installation-relative HTTPS validation. Test-only loopback fixtures remain behind the default-off `backend-test` feature. They do not replace ordinary browser, certificate or storage verification.

## Cancellation, storage and logout

Unconsumed observations retain cancellation ownership, including an authorized Poll reply already enqueued for the UI. A cancelled save cannot activate through a late provider completion. Exact creator/attempt/generation correlation keeps an old observation from retiring a fresh attempt or another account.

The ledger separately tracks durable eligibility, restrictive decisions, writer settlement and cleanup. Local closure is immediate; durable suppression, remote leave and provider deletion are distinct acknowledgements. A failed suppression commit remains visible and owned for an explicit retry. Do not label an unavailable store or uncertain writer as successfully cleaned.

Removal follows `RetireAll` → `Maintain` → `ForgetAccount`, with exact-item cleanup and honest pending states. `Maintain` cannot grant restoration eligibility. Local logout suppression does not claim server-side app-password revocation. Account content and outgoing work have a separate purge acknowledgement; report incomplete local cleanup rather than treating suppression as proof of deletion.

Private metadata uses bounded SQLite storage; credentials remain in the chosen provider. Private-file credentials use owner-only access controls and atomic generation files. The additive metadata migration preserves eligible credentials and stable account/navigation keys. Provider details and nonobvious transaction invariants stay beside the [ledger](../../crates/talk-adapters/src/identity/ledger.rs) and provider code.

On macOS the directory association uses the persistent volume UUID, effective
user and directory inode. The UUID is read from the validated directory's mount
through the system `diskutil` utility, with a three-second process lifetime and
32 KiB output bound. The old `st_dev` value identifies a current mount and is
not a persistent volume identity. An exactly matching legacy association upgrades
automatically. If only that legacy device value differs, explicit **Use account
storage again** recovery may associate the same user, inode and profile marker
with the current volume. A different user, inode, profile marker or already
persisted volume UUID is rejected even by that recovery action. The committed
provider identities, generations, eligibility, suppression and unsettled writes
are unchanged; ordinary restoration still validates the selected credential.

Connection diagnostics preserve allowlisted storage categories and profile-load
reasons, without paths, volume UUIDs, directory identifiers or database contents.

The ledger also records `connected_once` as historical metadata, independently of eligibility and runtime authority. The identity owner supplies a private avatar store only to durably eligible clients. Avatar files are separate from restoration metadata and credential files; store operations serialize with suppression and removal. The NextCloud content owner derives canonical avatar identities from the signed-in user, a room token or a recorded message author, so a fresh local picture is served before any room or message read. Encoded memory retention is 128 entries / 8 MiB per client; each account's disk store is a private directory of plain files holding at most 128 images, 1 MiB per image and 8 MiB of payload, plus small author records (message to avatar key, room-prefixed, retired with the room) and the self profile (display name and organization, each at most 256 bytes). Pictures older than seven days are refreshed from the server and, if it cannot answer, kept as the last known picture. Files share the private account folder's logout/removal lifecycle; the former SQLite picture files are deleted when the store is installed. A cached file records what this computer already received and is not new authority; a fresh room list retires files of rooms that disappeared or became restricted. Composition revalidates mounted avatars and the self profile after four hours, keeping the last image visible during transient failures and backing off for fifteen minutes. Unchanged bytes keep their decoded renderer allocation. Ordinary user pixels also survive an unmounted row within the 128-entry/8-MiB encoded and existing decoded-image budgets; restricted pixels do not outlive their authorized binding. Store failures fall back to memory. Profile-image storage remains separate from ordinary chat data and any content flow that requires a protected key.

The identity worker also owns the private per-account ordinary-content database. It serializes room metadata, recent history, member counts, eligible main/thread drafts and outgoing intent transactions under committed account eligibility. Composition owns optimistic overlays, dispatch order and reconciliation; storage never grants a live account or room permission. Stable account/room keys bind cached records across runtimes without persisting transport tokens. Restricted or unknown-policy content and session-only accounts remain in memory. [AUTH-009](../product/accounts.md#auth-009) and [MSG-003](../product/messaging.md#msg-003) define storage bounds, retention, staging and cleanup; cache eviction cannot discard an intent. A durable dispatch marker precedes a possible network effect, so process death cannot convert uncertainty into safe retry.

Only the [authoritative rejection classifier](nextcloud-contract.md#i2-auth-rejection-classifier-01--authoritative-rejection) can confirm account loss. Ordinary room denials, malformed probes and transient failures do not revoke an account. Confirmed loss closes the shared account scope before a later completion can re-enable it.

## Limits and verification

The facade, runtime queues, provider workers and HTTP runtime all have explicit finite capacities; their constants and admission behavior are documented in the owning code. No caller may create another runtime to evade capacity, recreate an account to erase uncertain writes, or treat process exit as proof of provider settlement.

Linux real-server/browser, private-file and isolated Secret Service checks cover their named paths. Native macOS/Windows providers, sleep/wake, all device combinations and complete performance qualification remain separate. See [current limits](../limitations.md), [account requirements](../product/accounts.md) and the [development guide](../development.md) for supported checks.
