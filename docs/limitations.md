# Known limitations

The alpha does not yet implement every feature of the official Talk clients.
Availability follows the server's capabilities, configuration and room policy.
For individual operations, see the [feature inventory](compatibility/feature-inventory.md).

## Servers and calls

- Exercised server baselines are NextCloud 34.0.5 / Talk 24.0.6 and NextCloud
  35.0.2 / Talk 25.0.6. HPB checks use Talk 25.0.6, signaling 2.1.1 and Janus 1.3.0.
- TURN over TCP/TLS is unavailable in the pinned async engine; live TURN
  credential rotation is not integrated into established application peers.
  The engine’s async `peer_connection/transport/turn_relayer.rs` rejects
  secure TURN and non-UDP TURN URLs before allocation; parsing TCP/TLS URLs in
  the core is insufficient. Supporting them requires an async transport with
  TCP/TLS connection ownership, certificate validation and TURN stream framing,
  followed by forced-relay media qualification. Relay-required calls do not fall
  back to host candidates.
  UDP relay checks do not establish connectivity through every enterprise network.
- Screen streams use separate internal-signaling or HPB/Janus connections.
  One remote screen is admitted, with VP8 decode up to 1920×1080; it takes
  priority within the server's video limit. Cameras retain their smaller decode
  profile. Unsupported/failed remote screen decoding retires that stream.
- Linux publication uses the ScreenCast portal's per-session source selection
  and PipeWire/GStreamer, without persisted sources or system audio. Sources
  with missing geometry, odd dimensions or dimensions above 1920×1080 are
  unavailable before capture admission. A matching desktop portal backend and
  `gstreamer1.0-pipewire` are required; Xvfb alone supplies neither. The software
  compositor probe uses an explicitly patched portal backend for SHM-only
  capture; unmodified compositor/picker and permission-revocation qualification
  remains necessary.
- macOS publication uses the bundled ScreenCaptureKit helper and the system
  content-sharing picker, with a three-buffer source queue, 1080p output and
  10 fps. It needs an Apple SDK build and native qualification, including picker
  cancellation, window closure, permission revocation and packaged identity.
  Linux compilation does not qualify this path. Windows publication and system
  audio remain unavailable. H.264 screen negotiation is not implemented; VP8 is
  the offered codec.
- Screen selection ends on call/control disconnection and cannot automatically
  restart after recovery. Stale source callbacks cannot publish into a new share.
  A call admits at most 32 retired remote screen negotiations; further remote
  screen offers are ignored until a new call. Concurrent screen broadcasters
  and conference-scale resource/performance qualification remain outside the
  supported profile.
- HPB socket resume preserves mute/camera intent and requires fresh policy before
  publication resumes. ICE recovery has a finite retry budget and a 15-second
  peer recovery deadline. A rejected HPB resume permits one fresh, scoped
  admission after cleanup, retaining mute/camera intent and requiring the call
  to still exist. Subscriptions also wait for HPB in-call readiness; an explicitly
  rejected subscription can retry within the same bounded recovery window and
  requires renewed policy. Full
  network handover and Janus ICE recovery remain unqualified; other continuity
  loss requires deliberate rejoin. Internal long-poll transport/server failures
  have three bounded retries, explicit interruption/resume states and fresh policy
  reconciliation. Lost signaling is not an empty successful poll; abandoned,
  malformed or exhausted reads close the call.
  A severed internal poll can remain active on the server for up to 30 seconds;
  recovery waits for a valid replacement poll rather than claiming continuity
  from REST success alone.
- Full HPB process replacement with an existing Talk 25.0.6 web peer currently
  can fail automatic media recovery in the r35 lab. On a fresh hello the web
  client's REST call join can precede its new HPB room acknowledgement. HPB
  2.1.1 restores its subscription gate from backend in-call changes, separately
  from the participant list it broadcasts. The new web session can therefore
  appear in-call and publish while HPB still rejects subscriptions to it. The
  native client waits for its room acknowledgement, makes a fresh admission,
  publishes and requests offers using the current mapped peer identity; that
  does not repair the other session's HPB gate. The web client can also retain
  obsolete peer identities after its fresh hello. Native retries remain bounded
  and do not bypass server denial or mutate another participant's call flags.
  An explicit web-peer call leave/rejoin restores AV in the exercised case.
  This is separate from the qualified interruption/resume of a surviving HPB
  session; automatic replacement with this peer remains unqualified.
- Guest/federated media and required call E2EE remain unavailable.
- The call view has adaptive gallery, local focus and a speaking-level-based
  speaker view. The level is a short-lived indicator from received audio or
  authorized local microphone samples, not speech recognition or a
  connection-quality measurement. The dock is inside
  the main window; it is not a separate always-on-top window. Sidebar avatars
  come from the current call; other active rooms show the call badge until
  participant identities are available.
- The screen stage consumes the media owner’s scoped stream identities and frames.
  Raised hands and reactions are scoped to
  projected participants and require advertised server support; their signaling
  interoperability needs further validation.
- Moderator mute, end-for-everyone and local-user re-ring requests check current
  policy and session identity; call-only removal and federated re-ring are not
  offered. A request receipt does not establish that another client acted on it.
  Server recording controls require a configured backend, a fresh moderator
  grant and an ordinary room. They start video recording; audio-only selection
  and recording-file workflows are not implemented. A configured recorder has
  not been exercised. Selecting a different capture device stops its current
  publication and requires the user to enable it again. These controls and
  automatic speaker selection require native and server interoperability
  validation.

## Messages and background activity

- Favorite/archive, room notification-level and poll-vote changes now use the same
  bounded optimistic outgoing queue as text, thread and reminder intents. Unknown
  cached permission permits local staging only; dispatch requires fresh authority.
- Each account has one executing REST operation and at most seven admitted
  descriptors. User, visible-content and background lanes reserve admission and
  select queued work in that order, with FIFO within each lane. User work can
  interrupt disposable background reads; it cannot preempt mutations or call
  control. The application additionally bounds ordinary HTTP concurrency to eight,
  with separate call transport lanes. Two local data slots per account keep
  required outgoing/draft commits independent of the network queue. Queue waits
  expire after 30/20/15 seconds by lane; transfers retain their longer class limit.
  After repeated transport failures a versioned, jittered circuit cooldown pauses
  background avatars, link cards, presence and prefetch while foreground recovery
  can proceed. Four likely ordinary rooms can be prefetched, one at a time, in a
  60-second planning window; account changes cancel speculative work.
- A resume gap over five seconds or a changed local route/source address interrupts
  stale HTTP/HPB work and requests a fresh newest-page history snapshot before
  resuming long polling. Retained history stays visible during revalidation;
  old or failed completions cannot release the catch-up fence. A successful
  authenticated history or room-list read clears transient reachability feedback,
  without clearing an authentication failure or renewing permission. A bounded route observation runs every
  two seconds outside GPUI. This detects source-address and Linux route changes;
  a handover that preserves both is observed through transport failure/heartbeat,
  not an OS-specific network API. Recovery never renews media or room authority.
  Native physical-network and platform sleep/call continuity qualification remains
  separate from the controlled lab interruption.
- Account presence/custom-status changes are optimistic and retried within the
  current authorized session, one per account for at most five minutes. Ambiguous
  writes are read back before another change can be attempted. These ephemeral
  status intents are not restored after restart. Uncertainty checks never rewrite
  an already applied status or renew its expiry.

- Marking unread from a message requires a live-proven preceding message or an
  exhausted live history page proving the first message; cached-only gaps do not
  establish adjacency. This message action requires a connection. Room-level
  mark unread can be queued offline, but an ambiguous server response is not
  replayed automatically; the action can be requested again explicitly.
  Locating an unloaded first unread message reads at most four history pages,
  plus an exhaustion check, and reports an unavailable result if the boundary
  cannot be established within that bound.
- Threads use the Talk `threads` capability and current room permissions. The
  account-wide list contains the subscriptions the server returns for rooms
  already known to the account, in pages of 50. Its last page is retained for
  the session; the list is discovered again after restart. The account-wide
  endpoint excludes unfollowed threads, including ones previously participated
  in; see the upstream [thread service](https://github.com/nextcloud/spreed/blob/stable35/lib/Service/ThreadService.php).
  Talk 24/25 has no independent thread read marker, so a thread's unread
  indicator is inferred from the room marker and latest known reply. Each
  displayed reply page holds at most 50 messages. The thread composer shares
  the room composer's options menu, with uploads, NextCloud sharing, polls and
  voice attachments unavailable until a thread-targeted attachment workflow is
  supported. Typing signaling has no thread identity in the implemented contract;
  room typing is neither sent from nor displayed inside the thread pane. Replier avatars use available
  cached replies and the server's most recent activity summary; unopened
  threads may initially show only one replier.
  Sending thread replies can use the bounded outgoing queue when a prior
  capability observation permits it. Thread-page reads retry transient failures
  up to four times before offering an explicit retry. A thread observed during
  the current session can have its title or subscription changed offline; the
  bounded queue revalidates the room when reconnecting. Uncertain effects are
  checked against the server and are never replayed blindly. Thread settings
  themselves are not restored from disk as permission authority.
  The API creates a thread with a new root message; it does not expose an
  operation to convert an existing ordinary message into a thread. See the
  upstream [thread controller](https://github.com/nextcloud/spreed/blob/stable35/lib/Controller/ThreadController.php)
  and [chat controller](https://github.com/nextcloud/spreed/blob/stable35/lib/Controller/ChatController.php).
- Private replies require a supported server, an eligible local user author and
  permission to expose the source and open a direct conversation. Reactions and
  their author lists are capability and permission gated; author observations
  are bounded and may need a fresh connection.
- Markdown covers bounded CommonMark, strikethrough and task markers. Tables
  remain readable source; HTML and Markdown images do not execute or load.
  The composer supports direct editing of basic marks, headings, lists, task
  markers, quotes, links and code with hidden syntax and canonical Markdown
  serialization. Arbitrary Notion blocks, tables and embedded content are absent.
  Native rich-editor/link/IME interaction still needs platform qualification.
- Typing uses the active ordinary local room's internal or HPB signaling session
  only when typing privacy and current room policy permit it. Classified,
  channel, federated and unknown-policy rooms are excluded. Indicators are
  ephemeral and expire; they are not an offline history of activity.
- Direct-conversation presence and custom status are refreshed with the room
  list when the server enables `user_status`. Recent direct-room status may be
  restored for five minutes from the existing private, unencrypted ordinary-room
  cache; it never grants authority and is hidden when stale. The account menu changes availability and
  edits custom and predefined messages with expiry. Member status is loaded with the
  room's bounded participant list, and message-author status is shown when that list
  provides a matching identity. People cards use current room observations; calling
  a group member first resolves the direct room and checks its call permission. Native
  accessibility qualification remains separate from UI fixtures.
- Poll drafts are server-managed and require moderator rights and the advertised
  capabilities. Using a draft to create a poll keeps the draft. Voter details
  are shown only when the server supplies them; avatar initials are local fallbacks.
  Recently viewed poll results are retained in memory for up to five minutes
  across four rooms, subject to room policy; they do not survive restart and
  cannot authorize voting or closure. Poll publication and draft changes require
  a connection and await server confirmation; they are not queued across restart.
  Native semantic accessibility and full platform behavior remain unqualified.
- Voice recording is explicitly started, limited to two minutes, and encoded as
  16 kHz mono PCM WAV in memory. An unsent recording remains available during
  a temporary disconnection, but retires on scope or permission loss and does
  not survive restart. Incoming originals are retained only for their player and
  are not available offline. Playback accepts WAV PCM, Ogg Opus/Vorbis and supported
  MP4/WebM audio codecs up to 32 MiB. AAC playback uses a native decoder on
  Linux and macOS; other platforms and unavailable codecs offer download. The
  deterministic waveform placeholder supports pointer scrubbing and five-second
  arrow-key seeking; it is not derived from the audio. Native microphone, speaker,
  permission and mobile interoperability have not yet been qualified.
- The quick switcher searches loaded conversation names, up to 16 observed
  participant names per eligible room, and the server's user directory. Its
  participant index is session-only and is filled gradually when permitted.
  Sidebar unread and mention filters are stored per durable account. Eligible
  cached rooms retain previously reported mention flags; missing or changed
  observations remain unknown. Room notification edits are optimistic and
  reconciled with the server. An uncertain write requires a fresh room read.
- Open-conversation browsing requires `listable-rooms`. The Talk v4 listing
  does not expose a reliable member count before joining; protected or unknown
  password states cannot yet be joined through the directory.
- Account-wide message search requires the server's Talk message provider and
  supported filters. It shows at most two pages and only results for rooms in
  the currently admitted room list; offline suggestions use the selected cached
  history. Search dates use the local system timezone and locale formatting,
  including daylight-saving changes; date filters wait for timezone discovery.
  Complete document viewers and parts of moderation remain incomplete. Documents can
  open in the Cloud web view.
- Link cards resolve at most two HTTP(S) links per eligible message using the
  NextCloud references endpoint. Up to 64 URLs are held in a session cache for
  fifteen minutes; negative results expire sooner. Only installation-owned
  reference-preview image routes are fetched, with bounded decoded image sizes.
  Provider selection and arbitrary embedded web content are not implemented.
- Conversation creation supports local-directory invitations and the returned
  webinar, presentation and voice-room presets. Other special-purpose presets
  and non-user invitation sources are not offered in the creation flow.
- Native PDF preview on macOS renders the first page of files up to 4 MiB; it is
  not a complete PDF reader. Linux and Windows use server-provided document
  thumbnails. If the server's PDF preview provider is disabled, those platforms
  offer the Cloud web view and download. Other documents use compact file cards
  with explicit downloads. Online raster previews use originals up to 4 MiB,
  8192 pixels per dimension and 16 Mi pixels. GIF/WebP render a still frame.
  Thumbnail fallback uses the same source limits and retains only a small raster;
  larger or unsupported inputs require the Cloud web view or download.
- Attachment selection supports up to eight files and sequential upload/share,
  limited to 32 MiB each before any stricter server limit. Unsent paths are
  session-local and retire on a room/account change or after thirty minutes.
  Submission creates immediate session-local chat rows with inline upload
  progress; a reference-correlated own message replaces each row after sharing.
  Lost share acknowledgements are reconciled by reading history, never replaying
  that share. Original bytes and local upload paths remain session-only. Once upload completes,
  eligible publication intents transfer durably before the session owner releases
  their caption. The existing-file workflow uses the same durable publication queue.
  Local raster previews use the same bounded decoder and stay out of disk caches.
  File-list and image paste and incoming file drops need native qualification on
  each platform; file-list paste has been exercised on Linux X11. Caption and
  reply/thread/silent metadata require the corresponding server capabilities.
  Native outgoing file drags use AppKit file promises on macOS and still need
  Finder qualification; Linux and Windows use the Download action.
- The account Files browser keeps up to eight recently visited listings in memory
  for ten minutes, scoped to the account. It can show them while disconnected
  during the same session; they do not survive restart. Folder paging uses the
  server's pagination support when available, and sorting applies to the current
  page. Search and recent results are limited to 200; other DAV responses are
  bounded to 1,000 entries and a response-size limit. A server that rejects DAV
  search or favorites disables that section. Eligible existing-file publications are staged durably and reconciled by
  reference after restart; the browser listing itself remains session-only.
- The media gallery and local video player are implemented but await native GUI,
  real-server and packaged-runtime qualification. The player accepts MP4
  H.264/H.265 with AAC on macOS and WebM VP8/VP9 with Opus, where native decoders
  are available. Other containers/codecs use the Cloud web view or download.
  Inputs are limited to 32 MiB, 1920×1080, 30 fps and a 30-minute player lifetime.
  Playback starts paused. Offline viewing uses a cached thumbnail only.
- Background discovery has bounded capacity. Calendar integration, all mention
  and thread-notification policies, and platform-native incoming-call actions
  remain incomplete. Unknown notification eligibility stays suppressed.
  The direct-mention badge uses the server's boolean flag alongside the total
  unread-message count; the server does not expose a direct-mention count.
  macOS Notifications settings now separate local choices from observed OS
  authorization and offer an explicit first request or denial guidance. The
  main-process permission prompt and delivery through the bundled notification helper still require
  native Mac qualification; Linux fixtures and macOS compilation do not prove it.
  macOS window closing keeps the application services alive; native close/reopen
  and notification reception with a hidden window still need Mac qualification.
  Explicit quit stops background reception.
- A complete room listing and its eligible durable metadata are bounded to 256
  rooms per account, including order and unread observations. Larger complete
  server listings require a future paged-list contract; they are not silently
  truncated. Sidebar customization placements remain separately bounded.
- Session history retains up to 32 rooms with 100 messages per room. Eligible
  durable accounts also restore up to 32 ordinary-room histories with the newest 50 timeline messages each, member
  counts, read-only scalar room settings and deliberately staged outgoing intents from private, unencrypted
  storage. Up to four recently opened 50-message thread windows per stored room
  share the existing cache size and eligibility limits; older windows are dropped
  first when the account cache is full. Restricted, unknown-policy and session-only
  content stays in memory. Inactive rooms are not continuously mirrored in full. Unsubmitted ordinary-room main/thread drafts persist separately,
  up to 32 entries and 128 KiB serialized per account, with a 30-day expiry.
  Attachments and cross-room private replies remain session-local. Saving is
  asynchronous: a sudden exit before a write/clear commits can restore older
  text. Failed or ineligible saves show session-only retention; unsent text is
  never evicted to admit another draft. Small reencoded previews persist for up to seven days, with
  at most 24 entries and 4 MiB per account; original attachments are not cached.
- The outgoing queue is bounded to 64 intents and 512 KiB per account. It
  revalidates current server authority before dispatch. Read-only uncertainty
  checks have finite windows, including the latest 200 messages for send/forward
  correlation. An unresolved effect stays retained and is never blindly replayed.
  Scheduled creation has no operation correlation and may remain uncertain.
  Reminders are server-owned; there is no standalone local OS reminder fallback.
  The account-wide list and translated text are bounded session caches and do not
  survive a restart. Talk 24/25 returns at most ten upcoming reminders without
  pagination; a full result is marked potentially incomplete. A reminder for a
  room absent from the bounded room list is omitted with the same indication. Notifications without
  a structured message identifier are suppressed. Notification activation is
  available on Linux notification daemons with action support and via the macOS
  helper; native delivery and click-through still need platform qualification.
  Translation requires a provider with source-language detection and a target
  matching the interface locale; task-only translation providers are not used.
  Summary generation requires a configured server assistant and the server's
  unread threshold; provider availability varies.
- Validated location objects render from their geo identifiers and can open
  OpenStreetMap deliberately; the client does not create locations or fetch map
  tiles. Incoming federation invitations and text chat use the server proxy;
  federated calls, files and other room tools are unavailable, and multi-server
  interoperability needs qualification. Breakout switching closes an active call
  before moving; joining the target call uses the normal call flow. Clearing help
  requests and restoring existing assignment selections are not implemented.
  Breakout v1 assigns logged-in users; changing the room count or mode requires
  removing the existing breakout rooms first.
  Waiting calls recheck policy, but automatic lobby admission needs real-server
  qualification. The lobby start time is shown only when room details were loaded.
  Room-avatar uploads accept regular images up to 8 MiB and 16 million pixels,
  crop to a centered square and send a bounded PNG. Avatar and federation
  invitation changes require a connection; they are not queued across restarts.

## Platforms

- macOS publication requires a persistent signing identity: an internal alpha
  certificate pinned to the app identifier, or a Developer ID Application
  certificate verified against its Apple team. The internal mode provides no
  Developer ID, hardened runtime or notarization. Non-publishing CI packages and
  releases through v0.0.10 remain ad-hoc signed. Moving from an old ad-hoc build,
  or later changing the signer, can require a new Keychain/TCC approval. Permission
  retention across updates still needs native qualification; no trust settings,
  credential ACLs or TCC databases are changed. See the
  [first-launch instructions](install.md#first-launch-blocked-by-macos).
- Mac CI checks compilation, package structure, native library paths and
  signatures. Native login, permissions, devices, PDF rendering and update installation still
  need platform qualification. Windows is compile-checked only.
- Native credential storage may be unavailable or report cleanup pending.
  The explicitly selected private-file fallback is persistent but unencrypted.
- Local preferences saved by this version use schema 8; older clients cannot open
  those preferences.
- macOS profiles upgraded to the persistent volume association cannot be opened
  by v0.0.8 and older builds. A legacy profile whose mount device value changed
  needs the explicit storage reassociation action. Copied or replaced directories
  are not automatically repaired. Native macOS volume lookup and this recovery
  flow still need qualification on a Mac.
- Synthetic media and successful compilation do not qualify physical devices,
  every OS, semantic accessibility or whole-process performance and energy use.

Report reproducible problems without credentials, private server details or
message contents. Security issues follow [SECURITY.md](../SECURITY.md).

HTTP JSON ingress also has a 131,072-node structure budget, a 64-level depth bound and a 1 MiB response byte bound (previously 256 KiB). This accommodates complete 256-room metadata responses; typed endpoint bounds still reject oversized lists without granting reconciliation authority.

A status change with a relative expiry whose acknowledgement is lost remains uncertain: matching status text alone cannot prove that the requested timer was installed. It is not automatically written again.
