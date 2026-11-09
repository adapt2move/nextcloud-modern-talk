# Accounts and storage

Follow the [security contract](../quality/security.md), especially Q.SEC-03–05, and the [identity contract](../architecture/identity-contract.md) for transaction and crash-recovery details. Qualification limits remain in [known limitations](../limitations.md).

## Spec amendment — AMEND-AUTH-STORAGE-01

Normal sign-in is durable: prefer the native credential store, then offer an explicit **unencrypted private-file** fallback if it is unavailable or fails. No primary-password storage, master-password setup or silent session-only fallback. Legacy temporary-session recovery remains supported by the security contract.

## AUTH-001

Validate the HTTPS origin and installation base path, show the destination and keep credentials bound to it. Do not bypass certificate failures. Distinguish invalid address, network/TLS failure, maintenance, missing Talk and unverified versions.

## AUTH-002

Use cancellable, bounded system-browser Login Flow v2 for app credentials, with pending, expired, refused and restart states. Validate identity before activation; duplicate or late responses cannot create/replace accounts. Keep login name and server user ID distinct. Retry the same destination using its reserved identity; an existing identity leads to reconnect, not a duplicate. Never log login
secrets.

## AUTH-003

Try native storage automatically for a new account; preserve a committed provider on restore/reauthorization. Before writing, commit the exact backend-qualified intent and suppress restoration while unresolved. Stage new N separately from retained P. Durable success requires confirmed storage **and** committed eligibility for that account/provider/item/generation. Enumeration or write receipt
alone cannot authorize restore. Startup attempts each eligible binding once, sequentially within 60 seconds; cancellation retires it and failure stays actionable. A bounded OS access prompt remains pending, not a false missing-credential result.

## AUTH-004

Offer private-file storage, retry and cancel after native failure, explaining that the file is unencrypted. Require private per-user paths, verified ownership, POSIX directories 0700/files 0600 or owner-restricted Windows DACLs and atomic durability. Unsafe paths/permissions block I/O. These controls do not protect against the same user, administrators or copied backups. File fallback cannot
bypass an unresolved native writer. Legacy session-only activation first requires all-backend restore suppression; uncertain writes and residual cleanup stay visible. No late abandoned N can activate, and early absence cannot settle a live writer.

## AUTH-005

Distinguish missing, unavailable, locked, denied and corrupt storage. Offer explicit repair/retry without repeated background prompts. Migration is deliberate: confirm target storage and sole target eligibility before authorized source cleanup. Failure preserves the last committed disposition and retained P. Cancel-and-retain may restore only the explicitly validated exact P; recovery never
silently changes providers, overwrites retained credentials or promises cleanup before settlement.

## AUTH-006

Partition credentials, requests, drafts, caches, notifications and media by account, host and narrower room/call scope, including any guest namespace. Switching accounts preserves each draft and selection; it cannot retarget pending work or transfer an active call. Links and notifications resolve their account explicitly. Identical room tokens or display names on different accounts must never
collide.

## AUTH-007

Confirmed account authorization loss invalidates its runtime, closes authenticated admission and stops owned capture/publication and background work independently of remote leave, vault or metadata success. A room denial, proxy 403 or transient network failure is not account revocation. Suppress restore across all providers; report failed durable suppression while still stopping locally. Preserve
permitted drafts and other accounts. Validated reauthorization creates a new generation; it alone never retries an uncertain send, rejoins a call or starts capture.

## AUTH-008

Removal explains active work and draft loss, stops the account, commits all-backend restore suppression and attempts exact authorized credential/cache cleanup and remote token revocation. Local removal and remote revocation are separate outcomes. Retain nonsecret cleanup markers for unsettled writers, inaccessible items and failed deletion. Late writes cannot reactivate the account; failure never
claims erasure or confirmed restart suppression. User-saved downloads remain user-owned.

## AUTH-009

Credentials and content have separate owners and eligibility. Only eligible durable accounts with confirmed read access and understood ordinary, non-sensitive, non-classified room policy may persist private **unencrypted** content. Restricted, unknown-policy and session-only contexts stay in memory. A locked required content key never permits plaintext downgrade. Cached observations grant no live
authority.

Bound each account to 256 room records, 32 histories (50 newest messages and 50 pin references each; 4 MiB per record/8 MiB total), and a 16 MiB content database. Observations expire within 30 days or earlier on expiry/access loss. Drafts have separate limits: 32 main/thread entries, 128 KiB total, 8 KiB per record, 4096 UTF-8 text bytes and 30-day expiry. Persist only stable keys, text, target
IDs and save time; no attachments, local paths, quoted excerpts or private-reply context. Reject capacity without evicting unsent drafts; failed saves retain the editor and explain session-only retention. Abrupt exit can restore an older asynchronous draft.

Thumbnails: at most 24, 256 KiB each/4 MiB total, seven days, reencoded PNG/JPEG without metadata at no more than 320×240. Bind to the exact room/message/descriptor; no source URLs. Avatars have separate bounded account storage and 24-hour freshness. Detail-view previews (images up to 4 MiB and rendered first PDF pages) have their own account file: at most 128 entries and 96 MiB, thirty days, least recently opened first, served only after the message and room were read fresh and never for classified, sensitive or unknown-policy rooms. Restricted images stay in memory. Access loss, replacement and logout retire both pixels and late completions; failed cleanup remains retryable and
visible.

The [outgoing queue](messaging.md#msg-003) never expires with ordinary cache data. An already-uploaded publication intent may retain a server-relative path (4096 bytes), filename (255), caption (4096) and voice/reply/thread/silent options, but no original bytes, local path, transport URL, reusable share link or permission grant. Persist no credentials, room tokens, membership lists or media frames
in content storage. Settings/history order/member counts are observations only; a historical connected-account marker cannot undo logout suppression. Logout/removal purge managed content and queued work, reporting failure; cache-clear excludes saved downloads.
