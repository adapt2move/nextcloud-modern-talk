# ADR-003 — Identity, durable restoration and account loss

Status: proposed for `G.BOOT`, 2026-10-18. AUTH-003–008, Q.SEC-02–05 and V.AUTH.STORE/LOSS are normative. **No Login Flow, vault, SQLite database or recovery worker exists yet.** Core contains only nonsecret contract types and narrow cleanup/admission checks.

## Identities and authority

The identity adapter owns browser authorization, pending secrets and vault/metadata transactions. It serializes transitions per account; the application account actor owns authenticated admission. A profile-wide single-writer process lock plus transactional revision comparison prevents two processes from replacing the same account independently. Losing that lock closes admission for restoration mutations, never silently becomes a second writer.

| Identity | Meaning and invalidation |
| --- | --- |
| `AccountId`, `ProfileId`, `HostId` | Random local IDs; persisted nonsecret binding to canonical server base URL including installation path and canonical server user ID. Login name is stored separately from canonical identity. Two users or installations never share a key. |
| `RuntimeGeneration` | Validated in-memory authorization lifetime. Logout, replacement activation or confirmed account rejection permanently closes it. It is not a vault generation or a persisted reusable token. |
| `LoginAttemptId` | One pending browser flow/replacement attempt. Cancellation abandons this attempt without implicitly invalidating a deliberately retained valid older runtime. |
| `VaultGeneration`, `OperationId` | Immutable attempted item identity and provider-operation ledger identity. New save attempts never overwrite old generations. Timeouts/cancellation do not settle a provider operation. |
| `CallGeneration` | One local join/incoming acceptance lifecycle; account scope and conversation-host scope are embedded. Reauthorization cannot reactivate it. |

Future item identity is application namespace `org.adapt2move.nextcloud-modern-talk` + profile + account + secret kind + schema version + vault generation. Labels contain no password, token, invitation credential or server secret. macOS targets application-owned generic-password Keychain records, with synchronization off where supported and access/signing policy qualified. Windows targets user-scoped Credential Manager; Linux targets a qualified Secret Service provider. No platform qualifies by name alone. [Q.SEC-03 primary platform sources](../../quality/security.md#qsec-03--os-vault-contract).

Only adapter-private code can materialize an app password or guest/signaling token. Worker operations use `AccountScope`; the identity owner lends the secret directly to a validated, origin-bound transport operation and releases its temporary buffer. Core, UI, diagnostics, task descriptors and media control receive no secret. Session-only is an explicit user decision; no plaintext preferences, token environment variables, encrypted-file-with-adjacent-key or mock vault can substitute for an OS vault.

## Nonsecret durability decision

Use a separate **SQLite restoration ledger** with a single writer, rollback journal `DELETE`, `synchronous=EXTRA`, and macOS `fullfsync=ON`, subject to native filesystem/power-loss qualification. No SQLite dependency is added at this gate. The selected future SQLite build/VFS and OS must report the effective settings and pass crash tests; unsupported flush guarantees leave restoration unqualified. The decision uses SQLite's [atomic commit model](https://www.sqlite.org/atomiccommit.html) and [documented pragmas](https://www.sqlite.org/pragma.html#pragma_synchronous), not a claim that writes are already durable on arbitrary hardware.

Schema version 1 consists of three logical records updated in one transaction:

- `AccountRestoration`: profile/account/origin/user binding, monotonic record revision, disposition (`Suppressed`, `Unresolved`, or `Eligible(exact generation)`), named retained generation and active attempt ID if any.
- `StoreOperation`: operation/attempt IDs, exact attempted generation, retained generation, phase (`IntentCommitted`, `Dispatched`, `SettledPresent`, `SettledAbsent`, `Abandoned`, `Activated`), settlement evidence and nonsecret error category. Abandonment and writer settlement are independent fields; one must not overwrite the other.
- `CleanupMarker`: exact account/attempted generation, reason, pending settlement/deletion state and last redacted provider code. The older retained item's removal needs a different explicitly authorized operation.

The journal and backups contain **no credentials**. They do contain sensitive account metadata: restrict filesystem access and exclude them from public diagnostics. Each commit compares the expected revision, completes its durable transaction and only then acknowledges the transition. Corruption/missing/inconsistent metadata is `RecoveryRequired`; vault enumeration cannot reconstruct eligibility. Deleting the ledger cannot make a surviving credential restorable. Validate schema and binding before any vault resolution.

## Operations and crash ordering

The worker-side `IdentityPort` exposes the semantic commands below. Its internal journal/provider seams remain private. `Resolve(AccountScope)` validates access; it never returns a secret through core. Reports distinguish persistence mode, exact generation, settlement and repair guidance. The unconfigured adapter returns an error for every command.

| Operation | Ordered action, acknowledgement and crash result |
| --- | --- |
| **Stage** | Validate live authorized attempt and distinct `N != P`. Commit intent and `Unresolved` account disposition **before** dispatch. If commit fails, dispatch nothing. Mark dispatch conservatively in the same pre-dispatch transaction; a crash between commit and actual call may overstate uncertainty safely. Call provider create for exact `N`, never update `P`. |
| **Settle** | Provider's definitive completion records presence/non-write and settlement. A timeout/lost reply stays unknown even if a lookup currently says absent. Persist the result without changing the user's disposition. A crash before this commit leaves `N` unresolved. |
| **Activate stored** | Revalidate the current noncancelled attempt, confirmed accessible `N`, canonical account binding and selected persistence intent. Commit `Eligible(N)` and activation first; only then issue a fresh runtime generation and display stored success. Retirement of `P` is separate authorized work. A crash after commit may restore `N`; before commit it cannot. |
| **Resolve** | Load intact committed account metadata; require exact `Eligible(N)`, applicable account validity and accessible matching item. Do not search for “some working item.” Resolve within the identity owner for the intended operation. Locked/denied/missing state yields structured recovery, not automatic fresh login success. |
| **Session-only** | Atomically mark the attempt abandoned, create exact-`N` cleanup marker and commit account-wide `Suppressed` covering **all** generations. Only then activate the new in-memory runtime. `P` remains untouched but ineligible. Unknown prior writes show `SessionOnlyCleanupPending`, never confirmed memory-only storage. |
| **Cancel and retain** | Invalidate attempt immediately. On deliberate retain choice, revalidate older runtime/account and commit `Eligible(P)` plus abandoned-`N` cleanup marker. Only this committed choice can restore `P` while `N` cleanup remains pending. If this commit fails, the last committed disposition governs; a prior unresolved intent continues to suppress automatic restoration. |
| **Reconcile** | Resolve operation settlement and exact-generation presence separately. No new store overlaps an unresolved writer. Late completion of abandoned `N` may update settlement/cleanup, never activate or change suppression/retention. |
| **Delete abandoned attempt** | After no-writer settlement is proved, delete only exact `N` in this profile/account. Confirm absence and commit cleanup completion. Repeated delete/absent is idempotent. Never wildcard-delete or treat `P` as cleanup collateral. A crash before final commit repeats reconciliation. |

macOS Keychain item APIs provide item results; **no durable operation-ID/settlement query is assumed**. A synchronous worker return can supply settlement while the process lives. If the process dies during an OS write, item lookup alone is insufficient to prove no late writer can recreate it. Until a platform-qualified recovery barrier establishes settlement, keep the cleanup marker and warning, preserve `P`, suppress restoration and require explicit recovery. Killing a worker or observing early absence is not that barrier. Automatic replacement/cleanup on such a path stays unsupported if qualification cannot establish it. This limitation affects availability, not the suppression rule; do not invent a Keychain transaction API.

If durable suppression cannot commit, do **not** activate a replacement under a session-only promise. Offer repair/retry or cancel-and-retain; cancellation still invalidates the attempt locally. If a secret cannot be read, session-only requires fresh authorization or a valid existing in-memory session. No cancelled/failed write may trigger a later automatic save. An explicit save uses a new generation after prior reconciliation.

Required provider categories are `Missing`, `Locked`, `AccessDenied`, `Corrupt`, `TemporarilyUnavailable`, `Unavailable`, `Unsupported`, `Cancelled`, `Other`, with operation/provider/account/retry/native-code metadata. Interpret native codes according to primary documentation: disallowed interaction is not proof of a locked vault. The UI gets a stable category and exact residual-state wording from AUTH-004, never raw native error descriptions or a wrong-password conclusion derived from vault failure.

## Account rejection classifier and fan-out

For both supported server families, the authoritative account probe is same-origin **`GET /ocs/v2.php/cloud/user?format=json`**, using only the intended app password, no ambient browser cookies and no redirects. The pinned route maps to `Users#getCurrentUser`. An OCS v2 unauthorized result uses OCS status **997** and HTTP **401** in the inspected response/middleware path. Sources for both releases are linked in [A-SRC-04](../source-evidence.md#a-src-04--account-probe).

`ConfirmedAccountProbe` requires an uncached, TLS-authenticated response from the configured canonical installation, a complete well-formed OCS failure envelope with that unauthorized status, the same still-current runtime scope, and a qualified endpoint mapping. A successful probe must return the expected canonical user ID. A changed ID closes admission as an identity mismatch and requires explicit repair; it cannot inherit the account's pending work. This confirms unusable authorization for the selected account, not its cause (revocation, deletion, admin action, etc.). Proxy HTML, missing OCS metadata, generic 401/403, TLS failures, captive portals and contradictory responses remain ambiguous. No success from the public capabilities endpoint proves account authorization.

| Evidence | Action |
| --- | --- |
| Confirmed account-probe rejection | Account-wide shutdown, below |
| Forbidden room operation, no `START_CALL`, lobby/room removal | Restrict/reconcile that action or room; no credential deletion or other-account invalidation |
| HPB ticket/session expiry or internal room session loss | Close affected call admission, reconcile session; not account rejection by itself |
| Timeout/DNS/5xx/rate limit/malformed/ambiguous authorization response | Mark degraded, schedule one bounded probe; retain credentials. In-flight media follows its independent policy expiry. |

The probe is coalesced with ADR-004's active-call refresh. Outside calls, an ambiguous action triggers at most one probe at a time with a 3 s deadline, then 2/4/8/16/30 s bounded jittered retry (maximum 30 s, respecting longer server retry guidance). No endless immediate login/prompt/retry loop. Failed probes never renew media authority. Exact unauthorized envelopes and successful identity binding must be live-tested on both releases before enabling this classifier; an unqualified form remains ambiguous.

At confirmed observation `t0`, the account actor synchronously closes authenticated admission and permanently invalidates the runtime. It broadcasts the same cancellation token to owned requests, transfers, sends, background/signaling work and rejects old login/policy/media completions. It closes the media publication gate in the same transition, then invokes priority stop independently of account cancellation. Publication/queue closure must complete by `t0 + 250 ms`; capture/device release by **`t0 + 1,000 ms`**, with watchdog/failure semantics in [ADR-004](004-policy-media-lifecycle.md#local-enforcement-and-failure). Remote leave/revocation, vault cleanup and metadata I/O cannot delay these local actions.

In parallel, identity commits account-wide restoration suppression, including older items. If persistence fails, runtime shutdown remains complete and the UI states restart suppression is unconfirmed. Permitted drafts and other accounts survive. Already sent data cannot be recalled; uncertain sends remain delivery-unknown. New validated authorization gets a new runtime and fresh gates, then requires deliberate send retry or rejoin. It cannot replay work, unmute, start a camera or resume sharing automatically.

Acceptance: all V.AUTH.STORE-01–06 and V.AUTH.LOSS-01–03, crash injection before/after each durable/provider boundary, two users/installations/profiles, failed suppression, late callbacks, provider lock/denial/signing changes and actual capture stop. Pure bootstrap tests substantiate only cleanup/admission algebra; native proof remains pending.
