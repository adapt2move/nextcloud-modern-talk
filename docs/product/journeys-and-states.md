# User journeys and shared states

This document defines end-to-end acceptance journeys and shared states. Each journey uses the detailed specifications; its presence here establishes no test result. The [feature inventory](../compatibility/feature-inventory.md) and [known limitations](../limitations.md) record implemented scope and limitations. Account, room, participant, and call state remain distinct.

## Journeys

### JNY-001

**First account.** Enter HTTPS server → validate → open system browser → complete server authentication/2FA → receive/validate device credentials → use preferred native storage or choose durable unencrypted private-file storage when unavailable → commit the required backend-qualified eligibility/suppression → load capabilities/conversations. Success: real data appears with truthful storage mode/cleanup status. File mode survives restart without requiring another login solely because the OS store is unavailable; normal sign-in offers no session-only path. A save failure stays in the storage step with retry/cancel; any existing temporary recovery state remains visibly temporary. An uncertain write stays visible and cannot restore automatically. Cancel/expiry invalidates the attempt; late completion cannot activate it. Reauthorization cancellation retains only deliberately selected valid P under AUTH-005. OS-store recovery never silently changes the committed choice. This follows [AMEND-AUTH-STORAGE-01](accounts.md#spec-amendment--amend-auth-storage-01); covers AUTH-001–005 and STATE-001–003.

### JNY-002

**Daily chat.** Pick account → filter/open room → load unread context → compose or restore draft → reply/attach as allowed → send → reconcile server message. Success: one canonical message appears in the intended room and another compatible client can read it. Offline/error keeps text and truthful delivery state. Covers CONV-001, MSG-001–005/010, CONTENT-001, DESK-007.

### JNY-003

**Create a guest conference.** Choose a returned preset → set name/invitees → select guest access separately from discovery → merge parameters and apply invariants/gates → review → create → reconcile returned room → copy/share its allowed link deliberately. Guest opens link → resolves identity/password/admission → views conversation → deliberately joins through conference prejoin. Success: membership and publication match policy; a classified preset never becomes a guest conference through forced settings. Covers CONV-002–007, CALL-004, MOD-001.

### JNY-004

**Accept a colleague's direct call.** Ring without capture → select Annehmen → show connecting call surface → resolve only necessary OS permission/consent/security steps → connect with permitted media. The “Bereit, wenn du es bist.” screen never appears. Cancellation during any step invalidates the attempt. Covers CALL-001–003/009/011 and CT-011–013.

### JNY-005

**Join and participate in a conference.** Voluntary join → device prejoin → current consent/security/admission → connected call → choose devices/layout, chat/raise hand/share screen as allowed → leave. Success: capture stops and room membership remains. A waiting participant sees lobby status rather than a failed call. Covers CALL-004–014 and MOD-001.

### JNY-006

**Moderate an event.** Inspect permissions → configure lobby/admission → manage participants → create/start/stop breakouts → start/stop recording if allowed → end call only with explicit authority/confirmation. Success: every affected participant receives the actual state, with consent and publication restrictions enforced. Covers MOD-001–004, CALL-009/010/014.

### JNY-007

**Work across accounts and hosts.** Receive a notification for account B while viewing A → open B's referenced room → verify federated host if applicable → retrieve context → act with B's permissions. Success: no A draft, secret, device setting, or cache becomes associated with B. Remote failure is identified independently. Covers AUTH-006, INT-001, DESK-002/007.

### JNY-008

**Recover from interruption.** Lose network/control connectivity or sleep while composing/in call → show stale/degraded/reconnecting state → preserve permitted unsent data and media intent → expire/suspend affected media grants within the approved bounds even if RTP survives → refresh policy and reconcile room/call existence → offer safe retry/rejoin. Confirmed account rejection instead shuts down that account locally under AUTH-007; reauthorization alone cannot resend or restart capture. Success: no duplicate send, invisible media activation, resurrected ended call, revoked-room write or old-generation callback revival. Covers AUTH-007, MSG-003/004, CALL-014, COMP-003, DESK-007/008.

### JNY-009

**Return to a message later.** Search or follow pin/reminder/thread → resolve message context → read/respond or show deleted/expired/denied state → optionally schedule a new message. Success: scopes and times are clear, and no stale excerpt overrides retention. Covers MSG-006/011–014 and CONTENT-007.

### JNY-010

**Safely remove an account.** Inspect active work → explicitly remove → stop account activity → commit all-backend suppression and attempt remote token revocation → remove authorized native/file secrets/cache → report pending exact-item cleanup or failed remote revocation. Success: no automatic restoration or background access; if durable suppression fails, local activity still stops and restart suppression is explicitly unconfirmed. Late writers cannot reactivate removed credentials. Covers AUTH-008/009 and DESK-008.

## Shared state contracts

### STATE-001

**Loading is bounded and cancellable.** Distinguish initial load, refreshing old data, pagination, and submitting an action. Keep useful confirmed content during refresh, labeled stale when necessary. Prevent duplicate staging of the same submission without trapping the user indefinitely. Deliberate queueable message actions project immediately and retain their own identities; a running network mutation does not disable unrelated message actions. Cancellation invalidates callbacks, not just the spinner. Acceptance: stale completions cannot overwrite a newer account/room/call selection.

Initial conversation and message collections use bounded, static skeleton rows in the space the content will occupy. They do not add an animated redraw loop or a waiting notice card. Routine refresh leaves confirmed rows mounted without replacing them with skeletons or inserting an updating banner; offline, denied and failed states retain their specific recovery explanation. The main conversation surface must not ask the user to choose a conversation while the initial room list is still loading.

Local admission contention does not create a notice, toast, waiting card or tooltip. Preserve input, the current selection and any existing actionable error when admission is rejected; rejection is not completion or pending work. Automatic observations remain eligible for their existing bounded owner retry. Only an admitted operation shows an in-place pending control or an initial-content skeleton, without changing control geometry or replacing confirmed content. Pending decoration owns no independent retry, timer or frame loop. Failed or uncertain effects keep their scoped recovery explanation; an admitted action that exhausts its pre-dispatch deadline remains an actionable failure.

### STATE-002

**Empty states state what is empty.** “Noch keine Unterhaltungen”, “Noch keine Nachrichten”, “Keine Suchtreffer”, “Keine Dateien”, and “Keine ausstehenden Einladungen” require a successful scoped response. Offer only authorized next actions. Acceptance: access denied, offline without a cache, and failed fetch are never presented as an empty result.

### STATE-003

**Unavailable, denied, and unknown are different.** Hide inapplicable feature creation, but explain a disabled action when it is needed to understand existing content or a user attempt. “Auf diesem Server nicht verfügbar”, “Keine Berechtigung”, “Durch Richtlinie gesperrt”, “Verbindung wird geprüft”, and “Noch nicht in diesem Client unterstützt” must not be conflated. Acceptance: unknown capability/policy never grants a sensitive operation, and known supported content is not erased just because its creation UI is deferred.

### STATE-004

**Network and server errors preserve intent.** Apply bounded backoff/cancellation, honor retry information, and distinguish network loss, maintenance, rate limits, server failure, unsupported operation, and remote federation failure. Reconcile uncertain mutations before retry. Acceptance: retry cannot duplicate a send/invite/dial-out or apply an obsolete form to a different account. A server upgrade requirement is not retried as a network error.

Show the observed connection cause during sign-in, beside the affected account in the account overview, and in the chat composer footer. Distinguish an unreachable server, timeout, HTTP access refusal, temporary server failure, rate limit and unexpected response. Preserve drafts and loaded history during a connection failure and prevent sends until the account connection is confirmed again. Clear the account warning after successful validation of that same account runtime. A room permission refusal does not mark other conversations or accounts as unavailable; a proxy's HTTP 403 does not revoke saved credentials.

### STATE-005

**Permission and lifecycle races invalidate actions.** Confirmed account rejection, room deletion, participant removal, role change, expiry, end of call and capability-hash change may arrive during a form or OS dialog. Revalidate before mutation and after uncertain responses; COMP-003 also requires bounded refresh for changes with no hash/event. Acceptance: AUTH-007 invalidates the rejected account's generation and stops local work/media independently of remote leave. A late approval/success cannot resurrect an obsolete object, renew expired authority, expose removed content or turn on media.

### STATE-006

**Privacy follows content into every surface.** Apply room/account restrictions to list previews, search, notifications, tooltips, pinned items, accessibility announcements, caches, diagnostics, clipboard/export, and external handoffs. Acceptance: sensitive/classified content does not leak through an alternate view even when the main timeline is protected. A hidden preview is not a claim that the server stores content with E2EE.

### STATE-007

**Success is observable and limited.** Confirm actions only when the responsible component has confirmed them: server acceptance, OS permission, provider write plus committed restoration eligibility, file save, or actual media connection. Acceptance: no “Gesendet”, “Sicher gespeichert”, “Aufnahme beendet”, “Verbunden”, or “Gelöscht” label is derived solely from a local click or animation. Credential-file status says “Auf diesem Gerät gespeichert (unverschlüsselte Datei)”; permissions do not justify an encryption/native-vault claim. Explain partial success and remaining work in the same context.
