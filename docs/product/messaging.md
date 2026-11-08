# Messaging and conversations

Use [shared acceptance rules](README.md#shared-acceptance-rules) and current [compatibility gates](../compatibility/README.md). The [limitations](../limitations.md) distinguish implemented tools from conditional or unavailable ones. Keep routine navigation calm; show advanced tools on demand.

## Conversations

<a id="conv-001"></a>

- **CONV-001:** One room identity owns chat, files, people and calls. Preserve drafts and confirmed rows through navigation/refresh; opening a row captures nothing. Reuse bounded account-scoped avatars without renewing authority. Empty, loading, denied and offline states stay distinct.
- **CONV-002:** Direct/group/public/note-to-self creation uses server identities and actual creation rights. Reuse returned direct rooms; reconcile ambiguous creation before retry and show partial invitations honestly.
- **CONV-003:** Merge returned default → preset → user changes → forced parameters, then apply restrictive server invariants and revalidate. Show effective locked settings and reconcile returned state; never infer privileges from localized preset names.

<a id="conv-004"></a>

- **CONV-004:** Enforce actual attributes: Channel forbids all calls and hides members from non-moderators (including cached reconstruction); Announcement inherits those rules and prevents ordinary-member leave. Preserved rooms allow only owner toggling and forbid delete/history/public-access/listability edits until unpreserved. Classified rooms are private, unlisted, forced sensitive, with no guests, SIP, recording, email/phone/federated invitations, unrelated object bindings, bots/bridges, summaries, private-reply exposure or attendee export. Client policy additionally excludes external excerpts, provider previews, clipboard/forward/export and durable content/drafts/thumbnails; authorized in-room use stays in memory. Sensitive/classified is not E2EE.
- **CONV-005:** Guest access and discoverability are separate. Required passwords precede publication and never enter links. Discovery grants neither entry nor call/publication rights.
- **CONV-006:** Separate joined-room filtering, people lookup, message search and open-room discovery. Join only after explicit selection and current admission checks; address books are never uploaded automatically. Signed-in public-room entry is not anonymous guest support.
- **CONV-007:** Identify invitation sources and partial failures accurately. Directory search/CSV preview never sends invitations. Member counts are dated observations, not permission to reconstruct hidden lists. Keep known counts during refresh.
- **CONV-008:** Resolve effective per-action permissions, role changes and owner transfer through supported APIs. Explain destructive override resets; denied edits restore confirmed values. Chat denial need not deny reactions.
- **CONV-009:** Favorites, archive, tags and personal sections organize the account's list without changing room semantics. Use confirmed server operations when supported; local fallback requires explicit unsupported evidence, never Unknown. Failed/uncertain edits preserve confirmed layout; no automatic replay.

<a id="conv-010"></a>

- **CONV-010:** Gate each metadata/lifecycle setting separately. Distinguish leave, delete, clear history, preserve and keep. Confirm classified keep through returned `classified_persist`; it retains classified restrictions/message expiry but excludes the room-retention sweep and later re-queuing. Preservation alone does not stop classified automatic retention. No exact deletion deadline or restoration of expired content is promised. Settings refresh cannot overwrite newer local edits or grant cached authority.

## MSG-001

Render safe native text, structured content and system events with stable scoped IDs. Paginate bounded history, deduplicate and preserve the reader's anchor; retain history while refreshing/offline. Unknown objects remain readable without executing HTML/scripts. Deleted/expired parents expose no retained quote/actions. Local times use the observed system timezone. Prefetch never selects, joins or
marks read.

## MSG-002

Provide multiline Unicode/Markdown composition, explicit send, emoji and discoverable send-key behavior. IME preedit never sends. Preserve text on limit/permission failure; unsupported formatting remains readable. Keep Markdown serialization separate from visible editing offsets. Links show validated destinations and require deliberate activation; HTML/images do not load external resources. Local
paste imports text without granting clipboard-export authority. Menus preserve draft, target and focus.

## MSG-003

Distinguish local queued, sending, server-accepted, failed, uncertain and common-read states. One account-owned queue holds at most 64 intents/512 KiB; overflow preserves input and accepted work. Eligible durable staging commits before clearing the matching unchanged editor; persist a dispatch marker before network effects. Explicit offline submissions may dispatch after fresh identity,
membership, policy, capability and target validation. Typing/restoration never submits. No cached grant or reauthorization releases an uncertain write. Reconcile read-only using bounded correlation (at most 200 recent messages); absence is not proof of nonexecution. Never blindly replay ambiguous POSTs, including uncorrelated schedule creation. Retain failed/unknown work for deliberate
retry/edit/discard; old receipts cannot clear newer text or replace newer intent state. See [AUTH-009](accounts.md#auth-009).

## MSG-004

Keep separate account/room/main/thread/reply drafts under storage eligibility and capacity rules. Navigation restores text without sending; deleted destinations require explicit recovery, never fallback to another timeline. Explain session-only retention and destructive draft loss where relevant. Attachment bytes and private replies stay session-local; draft eviction cannot make room for new work.

## MSG-005

Reply through canonical parent relationships and current replyability. Private replies name the recipient and require source exposure permission. Failure preserves context; only the matching unchanged accepted submission clears it, never a newer reply or IME.

## MSG-006

Use real server threads and `isThread` relationships, with separate drafts, root context, titles and subscriptions. Never simulate parity by grouping ordinary replies. Missing/deleted/inaccessible roots remain explicit; bounded discovery is not a complete account archive. Unsupported thread attachments/typing remain unavailable.

## MSG-007

Reactions have independent authority from chat. Project the newest admitted desired state; reconcile actual counts/own state and roll back only definitive failed intents. Old reads cannot erase newer changes. Offer keyboard-accessible quick actions/picker.

## MSG-008

Gate edit/delete by actor, type, authority and server windows. Keep edit text on failure; stale receipts cannot finish newer editing. Message/share deletion is not underlying-file deletion. Reconcile all previews, pins and excerpts after removal.

## MSG-009

Mentions use actual eligible actors and room-wide mention policy. Silent send, notification mute, typing privacy and presence remain independent. Emit no private typing events; expire indicators on idle/disconnect and retain metadata only where supported.

## MSG-010

Advance automatic read markers only through painted visible message ends in the current scope, monotonically until explicit mark-unread. Hidden windows, prefetch, search and notifications never mark read. Respect privacy; common-read receipts do not identify private individual readers. Cached gaps prove no adjacency for mark-unread.

## MSG-011

Search states its account/room scope and distinguishes server results from partial offline cache suggestions. Preserve queries on failure; inaccessible/deleted hits cannot reveal cached excerpts. Command/Ctrl+F and Escape preserve draft/call context.

## MSG-012

Pins reference canonical messages and obey expiry/access rules. Personal hide differs from shared unpin. Reconcile mutations and old reads without inventing confirmation; context navigation preserves the reader's anchor. Hidden pin events never justify reading unseen ordinary messages or crossing unproven page gaps.

## MSG-013

Reminders are server-owned personal actions with confirmed times and supported lists, not local timers or messages to others. A missing timestamp needs readback. Resolve DST ambiguity/skipped times explicitly; inaccessible targets expose no excerpt. Quitting does not cancel a confirmed reminder; notification delivery is separate.

## MSG-014

Schedule/edit/cancel through the supported server contract. Distinguish scheduled from sent; preserve text on rejection and reconcile uncertain creation/cancellation. No undocumented local timer fallback or claim of delivery after a mere request.

## MSG-015

Translation/summaries are deliberate, provider-dependent and policy-gated. Keep originals, label generated output and disclose processing where known. Never choose an external provider implicitly or mark unseen messages read because of a summary.

## MSG-016

Copy/forward requires current source exposure and destination authority. Name the account/destination and confirm; never forward automatically across accounts or impersonate the source author. Receipts belong to the destination. Cancel preserves the source composer; access loss retires clipboard/export authority.

## Files and structured content

- **CONTENT-001:** Explicit picker/paste/drop stages up to eight files; selection uploads nothing. Upload/share sequentially within 32 MiB per file and stricter server limits. Distinguish uploaded-but-unshared, failure and uncertainty; never reupload/replay an ambiguous share. Local sources expire on scope change/30 minutes.
- **CONTENT-002:** Browse/share only accessible account files. Preview audience and confirm before changing access; a rich object grants no download rights. Revalidate moved/revoked files and reconcile publication by reference.
- **CONTENT-003:** Use bounded native previews/gallery/downloads; no autoplay or arbitrary external fetch. Originals and thumbnails are labeled distinctly; offline offers eligible thumbnails only. Failed/denied previews cannot bypass access through fallback. Native file export confirms actual non-overwriting local publication, not animation.
- **CONTENT-004:** Preserve supported captions/reply/thread/silent metadata without duplicate caption sends. Unknown rich objects stay inert; locations require deliberate sharing and external-map activation.
- **CONTENT-005:** Voice recording needs explicit Record and microphone permission, with stop/cancel/play/send. Keep it an unsent in-memory draft; no empty/unsupported uploads or claimed mobile interoperability without checks. Stop on cancellation or authority loss.
- **CONTENT-006:** Polls use actual capability, vote/anonymity and draft/publish gates. Author/moderator Close ends voting and retains the published message; draft deletion is separate. Reconcile timeout/concurrent close; missing results are not zero votes. Never infer anonymous voter identities. Poll creation belongs in the composer.
- **CONTENT-007:** Expiry/revocation applies to every managed preview/cache/export path and late completion. Report failed cleanup; user-saved files cannot be promised remotely erasable.
- **CONTENT-008:** Native reference pickers require specific supported APIs and explicit selection. Web-only providers get a labeled safe browser handoff or unsupported state; no embedded JavaScript or automatic content transfer.

## Conditional integrations

- **INT-001:** Federation retains host/actor identity and local-versus-host capabilities, through the supported local proxy only. No credential forwarding or wrong-local-room fallback. Text support grants no file/call support; guest/federated media remains unavailable.
- **INT-002:** Treat bots/commands as attributed untrusted content. Room enablement needs supported authority and exposure review; classified rooms prohibit it. Never run local shell commands or require server bot secrets for ordinary room management.
- **INT-004:** Calendar meetings differ from message scheduling. No call capture on meeting save, guessed availability or local reminder promise. Optional Calendar notifications default off and require verified source integration.
- **INT-005:** File/event associations grant no broader file access. Public-share/video verification needs explicit human authorization; a video connection alone never releases protected access. Unsupported flows need an identified safe handoff.
- **INT-006:** Bridges require actual provider/API support and disclosure of external exposure. Secrets stay with their owner; a provider name grants no native protocol/E2EE parity.
- **INT-007:** External iframe call services are outside native embedding scope. Recognize the association; offer only a validated explicit browser route, otherwise unsupported. Never guess a native Talk join or execute iframe/postMessage code.
- **INT-008:** AI/automation output is generated, not privileged authority. Provider failure preserves sources; app installation alone enables no processing or automatic external disclosure.
