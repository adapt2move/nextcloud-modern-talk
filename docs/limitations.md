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
  can fail automatic media recovery in the r35 lab. The web peer publishes under
  a new signaling identity but also requests obsolete identities, and HPB denies
  native subscription requests despite advertised in-call flags. The native
  fresh admission remains blocked rather than bypassing that denial. An explicit
  web-peer call leave/rejoin restores AV in the exercised case. This is separate
  from the qualified interruption/resume of a surviving HPB session.
- Guest/federated media and required call E2EE remain unavailable.

## Messages and background activity

- Markdown covers bounded CommonMark, strikethrough and task markers. Tables
  remain readable source; HTML and Markdown images do not execute or load.
  The composer supports direct editing of basic marks, headings, lists, task
  markers, quotes, links and code with hidden syntax and canonical Markdown
  serialization. Arbitrary Notion blocks, tables and embedded content are absent.
  Native rich-editor/link/IME interaction still needs platform qualification.
- Typing indicators, voice messages, complete document viewers, account-wide
  content search and parts of moderation remain incomplete. Documents can open
  in the Cloud web view; poll drafts remain server-managed.
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
  that share. These pending attachment intents are not durable across restart.
  Local raster previews use the same bounded decoder and stay out of disk caches.
  Attachment paste and sending in reply/thread/silent modes remain unsupported.
  Native outgoing file drags use AppKit file promises on macOS and still need
  Finder qualification; Linux and Windows use the Download action.
- The media gallery and local video player are implemented but await native GUI,
  real-server and packaged-runtime qualification. The player accepts MP4
  H.264/H.265 with AAC on macOS and WebM VP8/VP9 with Opus, where native decoders
  are available. Other containers/codecs use the Cloud web view or download.
  Inputs are limited to 32 MiB, 1920×1080, 30 fps and a 30-minute player lifetime.
  Playback starts paused. Offline viewing uses a cached thumbnail only.
- Background discovery has bounded capacity. Calendar integration, all mention
  and thread-notification policies, and platform-native incoming-call actions
  remain incomplete. Unknown notification eligibility stays suppressed.
  macOS Notifications settings now separate local choices from observed OS
  authorization and offer an explicit first request or denial guidance. The
  main-process permission prompt and delivery through the bundled notification helper still require
  native Mac qualification; Linux fixtures and macOS compilation do not prove it.
  macOS window closing keeps the application services alive; native close/reopen
  and notification reception with a hidden window still need Mac qualification.
  Explicit quit stops background reception.
- Session history retains up to 32 rooms with 100 messages per room. Eligible
  durable accounts also restore up to eight ordinary-room histories, member
  counts, read-only scalar room settings and deliberately staged outgoing intents from private, unencrypted
  storage. Restricted, unknown-policy and session-only content stays in memory.
  Inactive rooms are not continuously mirrored in full. Unsubmitted drafts are
  session-local. Small reencoded previews persist for up to seven days, with
  at most 24 entries and 4 MiB per account; original attachments are not cached.
- The outgoing queue is bounded to 64 intents and 512 KiB per account. It
  revalidates current server authority before dispatch. Read-only uncertainty
  checks have finite windows, including the latest 200 messages for send/forward
  correlation. An unresolved effect stays retained and is never blindly replayed.
  Scheduled creation has no operation correlation and may remain uncertain.
  The reminder backend uses server reminders; there is no standalone local OS
  reminder fallback.

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
- macOS profiles upgraded to the persistent volume association cannot be opened
  by v0.0.8 and older builds. A legacy profile whose mount device value changed
  needs the explicit storage reassociation action. Copied or replaced directories
  are not automatically repaired. Native macOS volume lookup and this recovery
  flow still need qualification on a Mac.
- Synthetic media and successful compilation do not qualify physical devices,
  every OS, semantic accessibility or whole-process performance and energy use.

Report reproducible problems without credentials, private server details or
message contents. Security issues follow [SECURITY.md](../SECURITY.md).
