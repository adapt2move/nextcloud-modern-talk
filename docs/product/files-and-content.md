# Files, media, and structured content

This document defines file, media and structured-content behavior. The [known limitations](../limitations.md) record implemented scope and limitations. Protocol evidence: [chat/share, polls, integration, and capability sources](../compatibility/baselines-and-sources.md#protocol-sources). Rendering is native; HTML content never becomes an embedded browser surface.

## CONTENT-001

**Upload and attach local files.** Allow file picker, supported paste, and drag/drop into the active conversation. Preview filename, size, destination, and optional caption before sending. Check attachment configuration, quota/limits when known, and send permission. Resolve the account's attachment folder and supported per-conversation subfolder rules through server identity, not an unsanitized display-name path.

Acceptance: upload and sharing are distinct states. A completed upload whose chat share failed is shown as “Hochgeladen, noch nicht geteilt”, with a safe retry/share action; it is not uploaded again blindly. Partial batches show per-file outcomes and cancellation. Permission denial, no quota, deleted local source, path collision, disconnection, and server rejection preserve recoverable choices without claiming delivery.

The composer stages up to eight local files from the native multi-file picker,
supported native file-list or image clipboard formats, or an external file drop.
Compact removable chips show selection, upload, sharing
and per-file failure states; selection itself performs no upload. The ordinary
Send action sends the optional caption once, on the first confirmed share.
Files upload and share sequentially, with the existing 32 MiB per-file transport
limit and current server permissions. A definitive failed share retries the
already uploaded path. An uncertain dispatched write pauses the batch and has
no automatic or blind retry. Paths remain session-local, expire after thirty
minutes and retire on a room/account change. Reply, thread, caption and silent
metadata are sent only when their server capabilities and room policy allow them.
The account attachment destination can be chosen with the native folder browser;
the server's conversation-subfolder contract remains required for uploads.

## CONTENT-002

**Share existing NextCloud files.** Browse/search only the account's accessible files, show the selected object and sharing audience, then create the supported Talk attachment/share. A rich object is not evidence the user may download its contents. Respect guest/federated restrictions and current file/share permissions.

Acceptance: selecting a file never changes its sharing permissions until explicit send. If the file moved or access was revoked, fail safely and refresh. File-share credentials and private download URLs stay out of public diagnostics. Declining an access change leaves the file unshared.

The account-scoped native picker stages up to eight existing files in the
composer. Send creates a Talk room share for each item with a correlation
reference and supported caption, reply, thread and silent metadata. Pending rows
are replaced by correlated messages; an uncertain share is reconciled before
another attempt. Selection alone never creates a share.

## CONTENT-003

**View shared items and previews.** Offer a room-scoped files/media/links view when `rich-object-list-media` allows it. Display safe image/document/media previews, loading, unavailable preview, expired/deleted share, and download progress. Use native decoders/viewers where implemented; unsupported formats offer save/open externally with a clear handoff. Avoid autoplay and unexpected external resource loading.

Acceptance: one account's cached thumbnail cannot appear for another. Canceling a download leaves no misleading completed file. Opening a remote resource requires a deliberate action under the security policy. A missing preview falls back to file metadata, not a broken or blank message.

The shared-items view also opens the account's native Files browser. Its folder,
recent, favorite and search views provide metadata, thumbnails where available,
keyboard navigation, selection and explicit open/download actions. Folder pages
are fetched lazily where the server supports pagination. Recently viewed listings
are bounded in-memory account state, shown immediately while a refresh runs and
available during the session when disconnected. They are not persisted under
the room-content retention policy in [AUTH-009](accounts.md#auth-009).

Images and videos use compact thumbnail tiles without filenames. Size a loaded tile to its aspect ratio within 320×240 points, displaying the entire image without cropping or the former half-resolution display cap. Two landscape tiles can fit side by side in the 648-point grid. Consecutive bare media messages from the same actor within two minutes may form groups of up to eight; captions, replies, reactions, pins and other meaningful metadata keep their own rows. Grouping preserves every message's identity and permitted actions. PDF cards retain their filenames. Ordinary files such as JSON and ZIP use compact filename/type/size cards without thumbnail requests or a media-gallery entry. Missing or unsupported thumbnails fall back to those cards. Each attachment exposes a keyboard-focusable Download icon on hover or focus; clicking an ordinary file also offers Save. Loading and failed previews have visible labels; transient failures allow a bounded automatic retry and a deliberate retry action. When a loaded image changes a row's height, invalidate list measurements while preserving the reader's anchor.

An unchanged, confirmed thumbnail remains ready while its account, room, message and exact attachment descriptor remain current, within the existing pixel/count budgets. It does not expire after one minute merely because it is being displayed. A missing in-memory tile first uses an eligible local thumbnail, including online; a full local decode queue waits rather than redownloading the same image. Disk retention and exclusions remain governed by [AUTH-009](accounts.md#auth-009). Deletion, expiry, access loss and a replaced descriptor still retire the cached pixels.

Opening a tile presents a frameless, room-scoped media dialog on an opaque background, including during loading. Fit the complete media to the available width and height, clip its stage and keep the footer outside it. A Close button stays visible at the upper right. Previous/Next arrows, left/right keys and horizontal trackpad gestures navigate the loaded collection. The footer contains the filename, position, Download and authenticated Cloud web handoff. Navigation is bounded to a moving 128-item window over loaded history and shared/thread/pinned items; it does not fetch the account's entire file collection.

Online, supported raster images load their authenticated original directly. An available thumbnail may remain visible while the original loads, labelled as a small preview. If the original fails, label its unavailability and retain the permitted retry or external handoff; never present the thumbnail as a loaded original. Access denial suppresses that fallback. Raster inputs are limited to 4 MiB, 8192 pixels per dimension and 16 Mi pixels; supported formats are PNG, JPEG, GIF and WebP, with still-frame rendering. macOS PDF previews load the original, up to 4 MiB, and render the first page through the bounded native helper. Label that page and the observed page count. This is not a complete PDF reader. Other PDFs use supported server previews or an explicit external handoff; other document formats use file cards and explicit downloads.

Supported video originals use a separate 32 MiB input limit. The local player starts paused and accepts MP4 with H.264/H.265 and AAC on macOS, or WebM with VP8/VP9 and Opus, subject to the packaged native decoders. Playback is bounded to 1920×1080, 30 frames per second and a 30-minute lifetime. Unsupported containers/codecs offer the Cloud web view and download. The player never receives remote URLs or credentials. Closing, changing files, losing access or retiring the account immediately retires displayed pixels and requests native stop. A replacement waits for confirmed worker exit; an unconfirmed stop remains visible and cannot grant another player lease.

Offline, show only an eligible cached thumbnail, explicitly labelled as a small preview, without fetching an original or starting playback. The private cache in [AUTH-009](accounts.md#auth-009) survives restart under its size, retention and permission rules. Absence of a cached preview is an honest unavailable state.

File-share system messages retain their rich attachment cards. A missing inline preview may fall back to a bounded supported original, but permission or authentication denial must never trigger a fallback fetch. Every permitted item offers Download, with progress derived from transferred bytes and a clear pending/failed/cancelled state. Unknown totals stay indeterminate. A progress animation never implies successful publication of a partial file. Keep download progress and cancellation owned by the admitted transfer when a second download is rejected.

On macOS, dragging a permitted file card or media tile to Finder starts a
copy-only native file promise. Downloading begins only when the receiver supplies
its local destination. The application revalidates the exact captured attachment,
account and room, then uses the existing authenticated, staged, non-overwriting
download. Finder receives success only after actual local publication. One
export is admitted at a time, with a 120-second deadline; cancellation, access
loss and room/account retirement stop pending work. Only the filename is placed
on the native drag surface, never a remote route or credential. Linux and Windows
use the explicit Download action; outgoing native drags are not implemented
there. Native Finder interoperability still requires Mac qualification.

## CONTENT-004

**Captions and rich objects.** Render supported captions, references, file associations, locations, and other structured objects with source/actor identity. Preserve thread/reply/silent metadata only where that sharing API supports it. Unknown object types remain readable and never run executable content.

Acceptance: sharing with a caption does not accidentally send the caption twice. An unsupported rich object exposes a safe label/link when available, not fabricated content. Location sharing requires deliberate selection/OS permission where needed and never publishes background location.

## CONTENT-005

**Record and play voice messages.** Require `voice-message-sharing`, attachment permission, a supported native codec, and explicit microphone authorization. Start capture only after the record action; show a timer, level/state, stop, cancel, playback, and send. A recording remains an unsent draft until the user sends it. Recording a voice message is distinct from a call recording.

Acceptance: denied microphone access explains recovery without sending an empty file. Cancel stops capture and removes the app-managed temporary recording. Interrupted recording remains recoverable only under the cache policy. No unsupported format is uploaded while claiming cross-client playback; interoperability includes native-to-web/mobile and the reverse.

## CONTENT-006

**Create, participate in and close polls.** With `talk-polls`, render question/options, open/closed state, permitted votes/results, and anonymous-versus-visible voting semantics. Apply the server's multiple-choice rules. Moderator draft create/edit/publish uses `talk-polls-drafts` and `edit-draft-poll` where appropriate. Publishing is explicit.

Expose the same scoped poll flow from the composer's additional-content action and from conversation settings. Both entry points use current server capabilities and permissions; neither creates a local-only poll or fabricates results.

An open published poll offers an accessible **“Umfrage schließen”** action to its author (matching typed actor identity) or a participant with moderator authority, subject to current room/access/lobby gates. An ordinary voter who is neither cannot close it. Explain that closing ends voting and reveals the permitted final results. Submit once and show pending state; use the poll close operation, not message deletion. The [poll contract](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/poll.md#close-a-poll) uses `DELETE /poll/{token}/{pollId}` to close a published poll; its chat/share remains. Deleting a moderator draft is a separate action even though the [controller](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/PollController.php#L389-L469) uses the same route with a different outcome.

Acceptance: author and moderator can each close an open poll using keyboard/assistive technology; ordinary-voter, lost-authority, lobby and access denials keep the last confirmed state and explain the refusal. On confirmed closure, disable voting and Close, retain the poll/message and render authoritative results: hidden/anonymous results reveal only the aggregate, public results include only permitted voter details. Already closed (`400` in the pinned contract) or concurrent close triggers a state fetch and converges on the same closed poll, not a deletion/error-only dead end. Timeout/lost response shows completion unknown and reconciles before retry; if refresh finds it open, retain a deliberate authorized retry, and if refresh fails do not claim closed or zero votes. A concurrently rejected vote refreshes final results; a missing/deleted or inaccessible poll stays unavailable. Anonymous identities are never inferred from cache. Draft deletion/publication remains separate; failed publication retains the draft. [CT-010](../compatibility/test-matrix.md#ct-010) must exercise the native Close action itself.

## CONTENT-007

**Resolve content retention and permissions.** Expiring/deleted messages, revoked attachments, and classified room restrictions also apply to media browsers, download caches, voice drafts, previews, search, and clipboard/export actions. User-saved downloads cannot be promised remotely erasable; explain this distinction where relevant.

Acceptance: after the client observes expiry/revocation, it ceases serving protected cached previews and removes managed cache content under policy. An in-flight preview cannot reappear after access is removed. Cache cleanup failure is reported through diagnostics without exposing the content itself.

## CONTENT-008

**Integrate a native content picker where feasible.** Smart Picker/reference providers, third-party previews, and app-specific actions require an actual native/API contract. List supported providers and their destination before sending. For browser-only providers offer a clearly labeled external handoff if a safe route exists, otherwise mark unsupported.

Acceptance: the client does not embed provider JavaScript, pretend every web picker works natively, or silently submit conversation text to a provider. Disabled or failed providers leave the composer intact.
