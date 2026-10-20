# Files, media, and structured content

Status: **Planned**. Protocol evidence: [chat/share, polls, integration, and capability sources](../compatibility/baselines-and-sources.md#protocol-sources). Rendering is native; HTML content never becomes an embedded browser surface.

## CONTENT-001

**Upload and attach local files (P1).** Allow file picker, supported paste, and drag/drop into the active conversation. Preview filename, size, destination, and optional caption before sending. Check attachment configuration, quota/limits when known, and send permission. Resolve the account's attachment folder and supported per-conversation subfolder rules through server identity, not an unsanitized display-name path.

Acceptance: upload and sharing are distinct states. A completed upload whose chat share failed is shown as “Hochgeladen, noch nicht geteilt”, with a safe retry/share action; it is not uploaded again blindly. Partial batches show per-file outcomes and cancellation. Permission denial, no quota, deleted local source, path collision, disconnection, and server rejection preserve recoverable choices without claiming delivery.

## CONTENT-002

**Share existing NextCloud files (P1).** Browse/search only the account's accessible files, show the selected object and sharing audience, then create the supported Talk attachment/share. A rich object is not evidence the user may download its contents. Respect guest/federated restrictions and current file/share permissions.

Acceptance: selecting a file never changes its sharing permissions until explicit send. If the file moved or access was revoked, fail safely and refresh. File-share credentials and private download URLs stay out of public diagnostics. Declining an access change leaves the file unshared.

## CONTENT-003

**View shared items and previews (P1 basic; P3 complete browser).** Offer a room-scoped files/media/links view when `rich-object-list-media` allows it. Display safe image/document/media previews, loading, unavailable preview, expired/deleted share, and download progress. Use native decoders/viewers where implemented; unsupported formats offer save/open externally with a clear handoff. Avoid autoplay and unexpected external resource loading.

Acceptance: one account's cached thumbnail cannot appear for another. Canceling a download leaves no misleading completed file. Opening a remote resource requires a deliberate action under the security policy. A missing preview falls back to file metadata, not a broken or blank message.

## CONTENT-004

**Captions and rich objects (P1 safe rendering; P3 rich creation).** Render supported captions, references, file associations, locations, and other structured objects with source/actor identity. Preserve thread/reply/silent metadata only where that sharing API supports it. Unknown object types remain readable and never run executable content.

Acceptance: sharing with a caption does not accidentally send the caption twice. An unsupported rich object exposes a safe label/link when available, not fabricated content. Location sharing requires deliberate selection/OS permission where needed and never publishes background location.

## CONTENT-005

**Record and play voice messages (P3).** Require `voice-message-sharing`, attachment permission, a supported native codec, and explicit microphone authorization. Start capture only after the record action; show a timer, level/state, stop, cancel, playback, and send. A recording remains an unsent draft until the user sends it. Recording a voice message is distinct from a call recording.

Acceptance: denied microphone access explains recovery without sending an empty file. Cancel stops capture and removes the app-managed temporary recording. Interrupted recording remains recoverable only under the cache policy. No unsupported format is uploaded while claiming cross-client playback; interoperability includes native-to-web/mobile and the reverse.

## CONTENT-006

**Create, participate in and close polls (P3).** With `talk-polls`, render question/options, open/closed state, permitted votes/results, and anonymous-versus-visible voting semantics. Apply the server's multiple-choice rules. Moderator draft create/edit/publish uses `talk-polls-drafts` and `edit-draft-poll` where appropriate. Publishing is explicit.

An open published poll offers an accessible **“Umfrage schließen”** action to its author (matching typed actor identity) or a participant with moderator authority, subject to current room/access/lobby gates. An ordinary voter who is neither cannot close it. Explain that closing ends voting and reveals the permitted final results. Submit once and show pending state; use the poll close operation, not message deletion. The [poll contract](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/poll.md#close-a-poll) uses `DELETE /poll/{token}/{pollId}` to close a published poll; its chat/share remains. Deleting a moderator draft is a separate action even though the [controller](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/PollController.php#L389-L469) uses the same route with a different outcome.

Acceptance: author and moderator can each close an open poll using keyboard/assistive technology; ordinary-voter, lost-authority, lobby and access denials keep the last confirmed state and explain the refusal. On confirmed closure, disable voting and Close, retain the poll/message and render authoritative results: hidden/anonymous results reveal only the aggregate, public results include only permitted voter details. Already closed (`400` in the pinned contract) or concurrent close triggers a state fetch and converges on the same closed poll, not a deletion/error-only dead end. Timeout/lost response shows completion unknown and reconciles before retry; if refresh finds it open, retain a deliberate authorized retry, and if refresh fails do not claim closed or zero votes. A concurrently rejected vote refreshes final results; a missing/deleted or inaccessible poll stays unavailable. Anonymous identities are never inferred from cache. Draft deletion/publication remains separate; failed publication retains the draft. [CT-010](../compatibility/test-matrix.md#ct-010) must exercise the native Close action itself.

## CONTENT-007

**Resolve content retention and permissions (P1).** Expiring/deleted messages, revoked attachments, and classified room restrictions also apply to media browsers, download caches, voice drafts, previews, search, and clipboard/export actions. User-saved downloads cannot be promised remotely erasable; explain this distinction where relevant.

Acceptance: after the client observes expiry/revocation, it ceases serving protected cached previews and removes managed cache content under policy. An in-flight preview cannot reappear after access is removed. Cache cleanup failure is reported through diagnostics without exposing the content itself.

## CONTENT-008

**Integrate a native content picker where feasible (P4).** Smart Picker/reference providers, third-party previews, and app-specific actions require an actual native/API contract. List supported providers and their destination before sending. For browser-only providers offer a clearly labeled external handoff if a safe route exists, otherwise mark unsupported.

Acceptance: the client does not embed provider JavaScript, pretend every web picker works natively, or silently submit conversation text to a provider. Disabled or failed providers leave the composer intact.
