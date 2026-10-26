# Current implementation and limits

The native client connects accounts, conversations, messages, files, settings,
background activity and audio/video calls. The [installation guide](install.md)
and [known limitations](limitations.md) describe the installable profile; the [feature inventory](compatibility/feature-inventory.md)
separates implemented paths from remaining parity work.

## Connected application paths

- System-browser login uses NextCloud Login Flow v2. Accounts persist through
  automatic native storage for a new sign-in or an explicitly selected
  unencrypted private-file fallback after a storage failure.
  Restoration, logout suppression and cleanup share one account owner. Ordinary
  startup attempts each eligible saved credential once with its exact provider
  binding; failures remain explicit and background restoration preserves selection.
  Credential reads retain their original completion for up to 60 seconds while
  OS access is pending, with cancellation and bounded worker ownership. macOS
  `task dev` uses a persistent local development signature after `task dev:setup`;
  ad-hoc builds can still prompt again after recompilation.
  Account-scoped connection causes appear during login, in the account overview
  and in the chat footer. Failures preserve drafts and already staged work;
  eligible offline message actions can enter the local queue without claiming
  network success. Own queued rows show a clock with a waiting-for-connection
  tooltip, resuming through the existing scoped dispatch checks on reconnect.
- Compact grouped navigation includes a dedicated favorites group and active/archive views.
  Header actions open members and conversation settings; the title opens shared
  content. Messaging includes history, replies, edits/deletes, reactions, threads,
  pins, reminders, schedules, room search and explicit file upload/share/download.
  The main search includes the local-user directory. Scoped user avatars, room
  avatars and emoji icons supplement names; the footer identifies the current user.
  Avatars use canonical account-local byte caches with 24-hour expiry. Durable
  accounts retain bounded private profile-image files; session-only and restricted
  room images stay in memory. Decoded avatars survive eligible cached-room navigation
  and member-dialog refreshes, share authorized pixels and retain current navigation
  priority under eviction. Raster and emoji hits preserve the original source expiry.
  An authoritative member list supplies canonical user identities so fresh unrestricted
  profile pixels can satisfy the first member rendering without another content request.
  Restricted sources and destinations keep their own scoped checks. Busy admissions have a bounded retry queue; transient failures back off.
  Members use plain name/avatar/role rows with scoped action menus; invitation and
  CSV previews have separate steps and a bounded native file picker. Bots load on
  tab entry once online and authorized, retain known lists, use initial skeletons,
  and remain unavailable offline. Native dialog and server qualification is pending.
- Recently opened chats restore retained history immediately. Eligible durable
  accounts persist up to 100 ordinary-room metadata/count records and eight
  recent histories, each with up to 100 messages and 50 pins. Room settings retain
  eligible description/scalar observations for offline reading without permission
  grants. Settings revalidate quietly on relevant changes or server hints, with a
  60-second safety fallback through the existing observation schedule. The private,
  unencrypted content database has a 16 MiB ceiling; history payloads have a
  separate 8 MiB total limit and 30-day cache lifetime. Restricted, unknown-policy
  and session-only content stays in memory. Logout, access loss and expiry retire
  applicable content. Background prefetch still reads up to four eligible rooms,
  at most 50 messages each, without marking them read.
  Member counts retain their last observation and refresh after ten minutes,
  with a 60-second failed-attempt cooldown. Historical connection metadata
  survives restart independently of live authority.
- Deliberate message actions project optimistically through a queue of at most
  64 intents and 512 KiB per account. Eligible ordinary intents commit before
  their matching editor clears and survive restart. Dispatch validates current
  authority and writes a durable dispatch marker first. Definitive failures roll
  back the affected change and show a toast. Uncertain writes use bounded
  read-only reconciliation and remain retained without blind POST replay.
  Failed send/edit/schedule text remains in recoverable records with Retry,
  Edit as draft and Discard. Restoring a copy preserves the original record and
  does not overwrite an occupied editor or another retained reply/thread draft.
- Initial list/history loads use static skeletons; routine refresh keeps retained
  rows mounted. Message times use the system timezone and grouping uses its local
  calendar day with a two-minute adjacency window. Deleted rows retain their known
  author/avatar without actions or reactions. Own-message checks distinguish
  server acceptance from a supported common-read marker. Three quick reactions
  follow 12 persistent recent Unicode keys; a 36-emoji palette remains available.
  Custom emoji and complete reaction-author inspection remain unfinished.
  Call activity has status icons and red text for missed/unanswered calls,
  classified from canonical events and retained in ordinary cached history.
  Pins appear in a header banner with bounded flight/shred decoration, cycling
  and measured smooth navigation. A lower-right marker unpins directly.
  Redundant pin activity stays in canonical history and cursors while omitted
  from visible chat; receipt targets require observed page continuity.
  Reminder/schedule choices use local presets and handle daylight-saving gaps
  and repeated times. The reminder backend remains server-based. A separate
  forwarding dialog filters loaded same-account destinations, previews complete
  text and requires confirmation. Rich objects and truncated bodies remain
  unavailable for forwarding. Native pin/dialog interaction remains unqualified.
- Shared collection feedback distinguishes initial loading, refresh, offline,
  failure and confirmed empty results. Refresh keeps the retained timeline
  mounted, and explicit retry reloads the selected history. Stale failures
  cannot erase a newer successful read; thread failures remain thread-scoped.
- Shared content includes compact two-column media tiles, bounded same-actor
  groups and a frameless gallery with arrows, keyboard and trackpad navigation.
  Online raster/PDF previews request bounded authenticated originals; macOS PDF
  rendering remains first-page only. The local video player starts paused and
  uses explicit bounded MP4/WebM decode paths. Eligible accounts persist up to
  24 reencoded thumbnails, 4 MiB total, for seven days; offline viewers use only
  those small previews. Close/file/account transitions fence old completions and
  stop the scoped player. Unit, transport and synthetic codec checks cover parts
  of this path; native gallery interaction, real-server gallery workflows and the
  packaged video runtime remain unqualified. Cloud web handoff and byte-based
  downloads remain available. File-share messages retain attachment cards. Published
  polls support creation, voting, permitted results and closure from chat and
  conversation settings. Server poll drafts and complete PDF/office viewers remain
  separate unfinished work.
- Account scheduling reconciles room lists, nonselected-room messages and
  incoming calls. Authenticated live events invalidate observations; current
  server reads remain authoritative. Startup baselines do not replay old alerts
  or clear unread state, and unknown notification eligibility stays suppressed.
- Local appearance, English/German/Spanish, startup, notifications and device
  settings persist with rollback and explicit Retry. Account privacy and
  attachment settings use admitted protocol requests. Sequential permission setup
  advances on observed grants and retains a bounded account-specific restart
  handoff. Permission guidance and device checks never implicitly join a call.
- The independent [nextcloud-talk-api](../crates/nextcloud-talk-api/README.md)
  crate owns HTTP/OCS, typed operations and signaling. Application policy and
  device lifetime remain separate from the Rust WebRTC engine and GStreamer
  VP8/Opus codecs. Internal signaling and the qualified HPB/Janus profile use
  scoped call ownership, publication revocation and explicit reconnect.
- GitHub release checks run at a persisted randomized daily time or on demand.
  Background downloads become installable only after integrity and route checks.
  The user initiates installation outside calls; workers drain before complete
  package replacement and restart. Windows installation remains unavailable.
- Google Sans, Lucide actions, contextual tooltips, system emoji entry and finite
  control/dialog transitions are integrated. App dialogs share a 180 ms entrance
  with an eight-point panel rise; nested returns stay settled and close immediately.
  The chat header uses renderer-backed
  16-point backdrop blur with an 84% to 80% tint on Metal/Linux Blade; Windows
  uses an opaque white header with
  dark controls. Newly appended messages animate once and follow the
  end after a short delay only when the reader was already there. Reduce Motion,
  including an unknown observation, suppresses decorative animation without
  delaying input or focus.
- Text entry uses GPUI's native UTF-16 selection and marked-text contract,
  bounded composition undo and grapheme-aware display offsets. Dialogs keep
  deferred virtualized controls inert. Native macOS input-method and cursor
  behavior still requires runtime qualification on macOS.
- Message bodies, connection-error details and server addresses support selection
  and explicit clipboard copy within one text block, subject to room privacy
  policy. Selection across separate messages and a read-only text context menu
  are not implemented. macOS pointers use native system cursors; input carets
  remain GPUI-rendered.
  Carets blink while focused and visible, with a steady caret under Reduce Motion.
  macOS development builds read this preference directly without a bundled helper.
- Ordinary message text uses bounded CommonMark rendering with native emphasis,
  lists, headings, code and quotes. Safe web links open on deliberate activation
  after source and room revalidation. The composer edits formatted content with
  hidden Markdown syntax, bounded semantic undo and tentative native preedit.
  Visible text owns native offsets; canonical Markdown owns draft/submission
  values. Bold, italic and inline code have scoped keyboard shortcuts. Parsed
  timeline documents retire with their source/frame scope. Model and input
  regressions are covered; native link/rich-editor/IME interaction and semantic
  accessibility remain unqualified.

## Verification scope

Account and messaging adapters have run against disposable NextCloud 34.0.5 /
Talk 24.0.6 and 35.0.2 / Talk 25.0.6 servers. Ordinary Linux desktop checks cover
system-browser login, durable private-file storage, restoration, conversation
actions and messaging. Background discovery includes a real Talk 24 sequence
with nonselected-account messages, room creation/removal and incoming dismissal.
Focused content checks on both families cover authenticated avatars, published
polls with hidden/public results and denied closure, image/document previews or
Cloud fallback, and byte-exact downloads with progress, cancellation and
non-overwriting publication.

The media engine exchanged bidirectional VP8/Opus and SCTP between native peers,
with Chromium in both SDP roles, and through ordinary calls on both server
families. UDP STUN gathering and forced relay-to-relay UDP media were exercised;
bad relay credentials produced no host fallback. The Talk 25 HPB/Janus profile
includes browser media/status convergence, explicit reconnect and causal
publication revocation followed by confirmed call absence. These are bounded
synthetic-media checks, not physical-device, WAN or conference-scale qualification.

Linux runtime tests and macOS/Windows compile-only checks retain separate scope.
Native macOS/Windows login, permissions, devices, notification actions and update
installation are not established by compilation or package fixtures.

## Remaining scope

Full Talk parity is unfinished. Required call E2EE, guest/federated media,
screen/system-audio sharing in either direction remain absent. Unsubmitted
drafts remain session-local, and original attachment content is not stored in the
ordinary history cache. Uncertain send/forward checks cover at most 200 recent
messages; an unmatched effect stays retained. Scheduled creation lacks operation
correlation and may remain uncertain. There is no standalone local OS reminder
fallback. Typing, provider translation/summaries, voice messages and
complete native document viewers, account-wide content search and parts of
moderation and integrations remain incomplete.

The pinned Rust engine supports UDP TURN but not TCP/TLS TURN or live credential
rotation. New peers use fresh settings; unsupported or unusable required
transports fail explicitly. Functional media and bounded application queues do
not establish the full allocation, memory, energy or performance targets.

Calendar and complete mention/thread notification coverage remain unfinished.
Platform-native incoming-call actions, semantic accessibility, physical-device
quality and all-platform installation need further qualification. Historical
execution reports stay private; contributor documentation describes current
behavior and reproducible development workflows.
