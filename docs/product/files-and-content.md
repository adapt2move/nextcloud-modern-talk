# Files, media, and structured content

This document defines file, media and structured-content behavior. The [feature inventory](../compatibility/feature-inventory.md) and [known limitations](../limitations.md) record implemented scope and limitations. Protocol evidence: [chat/share, polls, integration, and capability sources](../compatibility/baselines-and-sources.md#protocol-sources). Rendering is native; HTML content never becomes an embedded browser surface.

## CONTENT-001

**Upload and attach local files (P1).** Allow file picker, supported paste, and drag/drop into the active conversation. Preview filename, size, destination, and optional caption before sending. Check attachment configuration, quota/limits when known, and send permission. Resolve the account's attachment folder and supported per-conversation subfolder rules through server identity, not an unsanitized display-name path.

Acceptance: upload and sharing are distinct states. A completed upload whose chat share failed is shown as “Hochgeladen, noch nicht geteilt”, with a safe retry/share action; it is not uploaded again blindly. Partial batches show per-file outcomes and cancellation. Permission denial, no quota, deleted local source, path collision, disconnection, and server rejection preserve recoverable choices without claiming delivery.

## CONTENT-002

**Share existing NextCloud files (P1).** Browse/search only the account's accessible files, show the selected object and sharing audience, then create the supported Talk attachment/share. A rich object is not evidence the user may download its contents. Respect guest/federated restrictions and current file/share permissions.

Acceptance: selecting a file never changes its sharing permissions until explicit send. If the file moved or access was revoked, fail safely and refresh. File-share credentials and private download URLs stay out of public diagnostics. Declining an access change leaves the file unshared.

## CONTENT-003

**View shared items and previews (P1 basic; P3 complete browser).** Offer a room-scoped files/media/links view when `rich-object-list-media` allows it. Display safe image/document/media previews, loading, unavailable preview, expired/deleted share, and download progress. Use native decoders/viewers where implemented; unsupported formats offer save/open externally with a clear handoff. Avoid autoplay and unexpected external resource loading.

Acceptance: one account's cached thumbnail cannot appear for another. Canceling a download leaves no misleading completed file. Opening a remote resource requires a deliberate action under the security policy. A missing preview falls back to file metadata, not a broken or blank message.

Images and videos use compact thumbnail tiles without filenames. Two tiles fit side by side. Consecutive bare media messages from the same actor within two minutes may form groups of up to eight; captions, replies, reactions, pins and other meaningful metadata keep their own rows. Grouping preserves every message's identity and permitted actions. PDF cards retain their filenames. Keep small server thumbnails at their useful display size instead of stretching them across the message column.

Opening a tile presents a frameless, room-scoped media dialog. Fit the media to the available width and height, with a background-free Close icon at the upper right. Previous/Next arrows, left/right keys and horizontal trackpad gestures navigate the loaded collection. The footer contains the filename, position, Download and authenticated Cloud web handoff. Navigation is bounded to a moving 128-item window over loaded history and shared/thread/pinned items; it does not fetch the account's entire file collection.

Online, supported raster images load their authenticated original directly. A failed original must not silently become a thumbnail. Raster inputs are limited to 4 MiB, 8192 pixels per dimension and 16 Mi pixels; supported formats are PNG, JPEG, GIF and WebP, with still-frame rendering. macOS PDF previews load the original, up to 4 MiB, and render the first page through the bounded native helper. Label that page and the observed page count. This is not a complete PDF reader. Other document/platform combinations use supported server previews or an explicit external handoff.

Supported video originals use a separate 32 MiB input limit. The local player starts paused and accepts MP4 with H.264/H.265 and AAC on macOS, or WebM with VP8/VP9 and Opus, subject to the packaged native decoders. Playback is bounded to 1920×1080, 30 frames per second and a 30-minute lifetime. Unsupported containers/codecs offer the Cloud web view and download. The player never receives remote URLs or credentials. Closing, changing files, losing access or retiring the account immediately retires displayed pixels and requests native stop. A replacement waits for confirmed worker exit; an unconfirmed stop remains visible and cannot grant another player lease.

Offline, show only an eligible cached thumbnail, explicitly labelled as a small preview, without fetching an original or starting playback. The private cache in [AUTH-009](accounts.md#auth-009) survives restart under its size, retention and permission rules. Absence of a cached preview is an honest unavailable state.

File-share system messages retain their rich attachment cards. A missing inline preview may fall back to a bounded supported original, but permission or authentication denial must never trigger a fallback fetch. Every permitted item offers Download, with progress derived from transferred bytes and a clear pending/failed/cancelled state. Unknown totals stay indeterminate. A progress animation never implies successful publication of a partial file. Keep download progress and cancellation owned by the admitted transfer when a second download is rejected.

## CONTENT-004

**Captions and rich objects (P1 safe rendering; P3 rich creation).** Render supported captions, references, file associations, locations, and other structured objects with source/actor identity. Preserve thread/reply/silent metadata only where that sharing API supports it. Unknown object types remain readable and never run executable content.

Acceptance: sharing with a caption does not accidentally send the caption twice. An unsupported rich object exposes a safe label/link when available, not fabricated content. Location sharing requires deliberate selection/OS permission where needed and never publishes background location.

## CONTENT-005

**Record and play voice messages (P3).** Require `voice-message-sharing`, attachment permission, a supported native codec, and explicit microphone authorization. Start capture only after the record action; show a timer, level/state, stop, cancel, playback, and send. A recording remains an unsent draft until the user sends it. Recording a voice message is distinct from a call recording.

Acceptance: denied microphone access explains recovery without sending an empty file. Cancel stops capture and removes the app-managed temporary recording. Interrupted recording remains recoverable only under the cache policy. No unsupported format is uploaded while claiming cross-client playback; interoperability includes native-to-web/mobile and the reverse.

## CONTENT-006

**Create, participate in and close polls (P3).** With `talk-polls`, render question/options, open/closed state, permitted votes/results, and anonymous-versus-visible voting semantics. Apply the server's multiple-choice rules. Moderator draft create/edit/publish uses `talk-polls-drafts` and `edit-draft-poll` where appropriate. Publishing is explicit.

Expose the same scoped poll flow from the composer's additional-content action and from conversation settings. Both entry points use current server capabilities and permissions; neither creates a local-only poll or fabricates results.

An open published poll offers an accessible **“Umfrage schließen”** action to its author (matching typed actor identity) or a participant with moderator authority, subject to current room/access/lobby gates. An ordinary voter who is neither cannot close it. Explain that closing ends voting and reveals the permitted final results. Submit once and show pending state; use the poll close operation, not message deletion. The [poll contract](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/poll.md#close-a-poll) uses `DELETE /poll/{token}/{pollId}` to close a published poll; its chat/share remains. Deleting a moderator draft is a separate action even though the [controller](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/PollController.php#L389-L469) uses the same route with a different outcome.

Acceptance: author and moderator can each close an open poll using keyboard/assistive technology; ordinary-voter, lost-authority, lobby and access denials keep the last confirmed state and explain the refusal. On confirmed closure, disable voting and Close, retain the poll/message and render authoritative results: hidden/anonymous results reveal only the aggregate, public results include only permitted voter details. Already closed (`400` in the pinned contract) or concurrent close triggers a state fetch and converges on the same closed poll, not a deletion/error-only dead end. Timeout/lost response shows completion unknown and reconciles before retry; if refresh finds it open, retain a deliberate authorized retry, and if refresh fails do not claim closed or zero votes. A concurrently rejected vote refreshes final results; a missing/deleted or inaccessible poll stays unavailable. Anonymous identities are never inferred from cache. Draft deletion/publication remains separate; failed publication retains the draft. [CT-010](../compatibility/test-matrix.md#ct-010) must exercise the native Close action itself.

## CONTENT-007

**Resolve content retention and permissions (P1).** Expiring/deleted messages, revoked attachments, and classified room restrictions also apply to media browsers, download caches, voice drafts, previews, search, and clipboard/export actions. User-saved downloads cannot be promised remotely erasable; explain this distinction where relevant.

Acceptance: after the client observes expiry/revocation, it ceases serving protected cached previews and removes managed cache content under policy. An in-flight preview cannot reappear after access is removed. Cache cleanup failure is reported through diagnostics without exposing the content itself.

## CONTENT-008

**Integrate a native content picker where feasible (P4).** Smart Picker/reference providers, third-party previews, and app-specific actions require an actual native/API contract. List supported providers and their destination before sending. For browser-only providers offer a clearly labeled external handoff if a safe route exists, otherwise mark unsupported.

Acceptance: the client does not embed provider JavaScript, pretend every web picker works natively, or silently submit conversation text to a provider. Disabled or failed providers leave the composer intact.
