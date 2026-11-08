# Known limitations

These are the limits of **v0.0.13 alpha**. Features follow current server
capabilities, configuration, room policy and permissions; the
[product requirements](product/README.md) describe intended behavior, not a claim
of complete parity with the official clients.

## Servers and calls

- Target families are NextCloud 34 / Talk 24 and NextCloud 35 / Talk 25. Lab
  baselines are 34.0.5 / 24.0.6 and 35.0.2 / 25.0.6; other patches and compatible
  backports are not automatically verified. Calls select `conversation-v4` and
  `signaling-v3` by capability rather than a major-version allowlist.
- **Call E2EE follows Talk's web protocol and is not yet proven in a live call
  with another client.** When `call-end-to-end-encryption` is advertised and not
  disabled, every audio and video frame is sealed before packetization
  (AES-128-GCM, HKDF-SHA256 keys, per-participant sender keys over Olm sessions,
  rotation on departure, ratchet on join) and frames that cannot be opened are
  dropped; nothing is sent in the clear. Checked: the frame format byte for byte
  against the web client's own WebCrypto output, the key exchange against Talk's
  JavaScript on libolm in both roles including rotation, sealed Opus/VP8 through
  the real RTP and decoder path, and the signaling transport (internal and HPB)
  in unit tests. **Not checked:** a call with the Talk web client, Android/iOS or
  an HPB/Janus deployment; mixed encrypted and non-encrypted participants (the
  web client shows no media from them, and so does this client); lost key
  messages beyond the 5 s request timeout. Participants are not authenticated
  beyond the server's session identity: there is no safety-number or key
  verification. Transport encryption is not call E2EE. Chat and file content are
  not end-to-end encrypted. Guest and federated media are unavailable. The call
  header shows a lock only while this client protects the call's media.
- The call card and local diagnostics distinguish missing policy evidence from
  a prohibition. Unknown required policy stays closed; failed reads retry without
  granting media authority. Ordinary rooms require applicable recording consent;
  classified rooms never submit recording consent or allow recording. Changed
  consent closes the old media owner and requires explicit rejoin.
- Internal signaling and HPB are implemented; the HPB lab profile uses Talk
  25.0.6, signaling 2.1.1 and Janus 1.3.0. TURN relay works over UDP and, through
  a loopback bridge, over TCP and TLS (`turns:`, typically port 443); the relay's
  certificate is verified by the operating system's trust store, so a company CA
  installed in the system is honoured, and an unverified relay carries nothing.
  TLS TURN was checked against a real coturn server with relayed audio and video
  decoding on both sides, not against a Talk deployment's own TURN. STUN over TLS
  or TCP and live TURN credential rotation in established peers are unavailable.
  The bridge is a separate TCP connection per local interface the engine
  allocates on, which a TLS-only TURN server may count against per-user quotas.
  Relay-required calls never fall back to host candidates.
- Recovery is bounded and revalidates policy. HPB resume retains microphone/camera
  intent, but full network handover and Janus ICE recovery remain unqualified.
  Full HPB process replacement with an existing Talk 25 web peer can leave its
  subscription gate stale; explicit web-peer leave/rejoin restored media in the
  exercised case. A native rejoin cannot repair the remote peer's state.
- Screen sharing uses VP8, admits one remote screen up to 1920×1080 and stops
  selection on call/control loss. It never restarts automatically after recovery.
  Multiple screen broadcasters and conference-scale performance are unqualified.
  Missing grid policy suspends remote video without blocking an otherwise allowed
  audio call.
- Linux capture requires a matching ScreenCast portal backend and
  `gstreamer1.0-pipewire`; Xvfb alone is insufficient. Sources need even dimensions
  and known geometry no larger than 1920×1080. Unmodified desktop picker/capture
  and revocation behavior remain unqualified. macOS uses the bundled
  ScreenCaptureKit helper and system picker at up to 1080p/10 fps; native packaged
  permission and lifecycle checks remain necessary. Windows screen publication,
  system audio and H.264 screen negotiation are unavailable.
- Recording needs a configured backend and fresh moderator authority; recorder
  interoperability, audio-only selection and recording-file workflows are not
  established. Raised hands, reactions, moderator controls, automatic lobby
  admission and physical devices need further native/server validation. Device
  switching stops publication and requires explicit enablement. The call dock
  stays inside the main window.

## Navigation, messages and offline use

- The sidebar has one Activity destination with unread, mentions, threads and
  reminders. Filters live in the conversation-list menu. Counts reflect bounded
  account observations; mention flags are not direct-mention counts. Connection
  status measures time since this client observed failure, not last successful sync.
- Signed-in users can discover and enter public rooms, including a same-server
  `/call/<token>` link pasted into open-conversation search. Entry joins the chat
  session before loading history, requests required passwords separately, and
  distinguishes lobby waiting from revoked access. Discovery needs `listable-rooms`;
  listing results have no reliable pre-join member count. Anonymous guest login
  and OS deep-link registration are unavailable.
- Complete room lists are limited to 256 rooms per account; larger lists are
  rejected rather than silently truncated. Session history holds up to 32 rooms
  with 100 messages each. Eligible durable accounts restore the newest 50 timeline
  messages per cached room and up to four recently opened thread windows.
  Inactive rooms are not fully mirrored. Restricted or unknown-policy content
  stays in memory.
- Ordinary main/thread drafts persist separately with a 30-day expiry, up to
  32 entries and 128 KiB per account. Writes are asynchronous; abrupt exit can
  restore older text. Attachments and private replies stay session-local. Failed
  saves show session-only retention; unsent drafts are never evicted to make room.
- The outgoing queue holds at most 64 intents / 512 KiB per account and checks
  fresh authority before dispatch. Uncertain effects use bounded read-only
  reconciliation and are never blindly replayed. Failed sends appear above the
  main composer with recovery actions, rather than in their former timeline
  position; the thread composer does not show failed records. Schedule creation
  can remain uncertain because it has no operation correlation.
- Conversations can be dragged onto group headings and other conversations in the
  sidebar. Server tags store membership only, so a drop assigns the target as the
  conversation's single custom group and cannot order rows inside it; local groups
  and sections keep an order. Dragging has no keyboard equivalent yet: the
  conversation's organize dialog remains the accessible route. Drag-and-drop was
  built and unit-tested but not exercised with a pointer in an automated check.
- Threads require the server's `threads` capability and actual `isThread` flag.
  Account-wide discovery lists server-returned subscriptions in known rooms and
  is session-only. Unread is inferred from room read markers. Reply pages hold
  50 messages; thread attachments and typing are unavailable. An ordinary message
  cannot be converted into a thread through the implemented API.
- Message-level mark unread needs a live-proven preceding message; cached gaps
  do not establish adjacency. Unread navigation searches a bounded number of
  pages. Account search shows at most two pages in admitted rooms; offline
  suggestions use cached history.
- Markdown supports basic formatting, lists, quotes, links, code and task markers.
  Tables remain source text; HTML and Markdown images do not execute or load.
  Rich-editor/IME and semantic accessibility behavior need native qualification.
- Account presence/status changes are session-bound and are not restored as
  pending intents after restart. Poll publication/draft changes and room-avatar
  changes require a connection. Reminders are server-owned: the list is
  session-only, limited to ten upcoming entries without pagination, and can be
  incomplete. Translation and summaries require compatible server providers;
  translated text is session-only. There is no local reminder fallback.
- Creation offers local-user invitations and returned webinar, presentation and
  voice-room presets. Other invitation sources and special-purpose creation
  presets are not offered. Classified/channel/preserved restrictions remain
  enforced without claiming complete administration parity. Federation supports
  incoming invitations and text through the server proxy; files, calls and other
  room tools are unavailable and multi-server interoperability needs validation.
  Breakout controls lack help-request clearing and restoration of assignment
  selections. Calendar integration and parts of moderation remain incomplete.

## Files and media content

- Select at most eight attachments, uploaded/shared sequentially with a 32 MiB
  per-file ceiling before stricter server limits. Original bytes and selected
  paths stay session-local and expire on scope change or after 30 minutes.
  Uploaded publication intents can persist; lost share acknowledgements are
  reconciled without replay. Original attachments are not cached for offline use.
- The Files browser caches up to eight listings for ten minutes in the current
  session; search/recent results stop at 200. Sorting covers the current page.
  DAV support depends on the server. File paste/drop was exercised on Linux X11;
  image paste and macOS Finder file promises still need native validation.
  Linux/Windows use Download for outgoing files.
- macOS PDF preview renders only the first page, up to 4 MiB. Linux/Windows use
  server thumbnails. Raster previews are bounded; GIF/WebP show still frames.
  Unsupported or larger content offers browser handoff or download. Link cards
  depend on the server's reference providers and fetch no arbitrary embedded web
  content. Locations can open OpenStreetMap deliberately; map tiles are not loaded.
- The gallery/video player needs native, real-server and packaged-runtime
  validation. Supported local playback depends on available codecs and is limited
  to 32 MiB, 1920×1080 and 30 fps; offline viewing uses a cached thumbnail.
- Voice recording is explicit, in-memory, at most two minutes of 16 kHz mono PCM
  WAV. Unsent recordings do not survive restart; incoming originals are not
  available offline. Playback codecs vary by platform; unsupported formats offer
  download. Incoming waveforms are illustrative, not decoded audio. Physical
  devices, permissions and mobile interoperability remain unqualified.

## Notifications

- While running, healthy accounts poll message/reminder and incoming-call
  notifications every three seconds. Server cron/delivery, request time,
  backpressure and retries add latency. Initial observation and gaps of at least
  60 seconds establish a silent baseline. Explicit quit stops reception;
  `notify_push` websocket hints are not implemented.
- Ordinary message banners require a background window and known eligibility.
  Unknown policies stay suppressed; not all mention/thread policies or native
  incoming-call actions are implemented. Fresh `occ notification:test-push --talk
  USER` diagnostics use fixed local text, can appear in the foreground and need
  no mobile push registration. General administrator notifications are excluded.
- macOS separates saved notification choices from observed OS authorization.
  Native permission prompts, hidden-window reception and click-through still
  need Mac validation. Linux activation requires a notification daemon and
  confinement policy that allow actions; banner delivery alone does not prove it.

## Platforms and local storage

- macOS is the primary target; Linux is a development/packaging target; Windows
  is compile-only. Compilation/package checks do not qualify native login,
  Keychain, permissions, devices, PDF rendering, update installation or VoiceOver.
- macOS alpha publication uses a persistent signing identity. Internal signing
  is not Developer ID signing or notarization. Moving from older ad-hoc builds
  or changing signer can require new Keychain/TCC approval; permission retention
  across updates still needs validation. See [installation](install.md).
- Native credential storage can be unavailable or leave cleanup pending. The
  explicitly selected durable private-file fallback is **unencrypted**, as are
  eligible content caches; providers never migrate silently.
- Preferences use schema 8 and content caches schema 4; older clients may reject
  them. macOS profiles with persistent volume association cannot be opened by
  v0.0.8 or older; changed legacy mount identity needs explicit reassociation.
- Synthetic media, screenshots and builds establish neither physical-device
  support nor whole-process performance, energy use or semantic accessibility.
  Application queue bounds are not hard decoder/OS/GPU memory ceilings.

Report bugs with synthetic reproduction data. Disclose security issues through
[SECURITY.md](../SECURITY.md).
