# Known limitations

The alpha does not yet implement every feature of the official Talk clients.
Availability follows the server's capabilities, configuration and room policy.
For individual operations, see the [feature inventory](compatibility/feature-inventory.md).

## Servers and calls

- Exercised server baselines are NextCloud 34.0.5 / Talk 24.0.6 and NextCloud
  35.0.2 / Talk 25.0.6. HPB checks use Talk 25.0.6, signaling 2.1.1 and Janus 1.3.0.
- TURN over TCP/TLS and live TURN credential rotation are unavailable in the
  pinned engine. Relay-required calls do not fall back to host candidates.
  UDP relay checks do not establish connectivity through every enterprise network.
- Guest/federated media, required call E2EE, screen/system-audio sharing and
  conference-scale qualification remain outside the supported profile.
  A remote screen offer ends the unsupported call. Permission guidance is not
  a sharing engine.

## Messages and background activity

- Markdown covers bounded CommonMark, strikethrough and task markers. Tables
  remain readable source; HTML and Markdown images do not execute or load.
  The composer shows and styles the original Markdown characters. Full rich-text
  editing with hidden syntax is not implemented. Native Markdown/link/IME
  interaction still needs platform qualification.
- Typing indicators, voice messages, complete document viewers, account-wide
  content search and parts of moderation remain incomplete. Documents can open
  in the Cloud web view; poll drafts remain server-managed.
- Native PDF preview on macOS renders the first page of files up to 4 MiB; it is
  not a complete PDF reader. Linux and Windows use server-provided document
  thumbnails. If the server's PDF preview provider is disabled, those platforms
  offer the Cloud web view and download. Office document previews also depend on
  the server's providers. Online raster previews use originals up to 4 MiB,
  8192 pixels per dimension and 16 Mi pixels. GIF/WebP render a still frame.
- The media gallery and local video player are implemented but await native GUI,
  real-server and packaged-runtime qualification. The player accepts MP4
  H.264/H.265 with AAC on macOS and WebM VP8/VP9 with Opus, where native decoders
  are available. Other containers/codecs use the Cloud web view or download.
  Inputs are limited to 32 MiB, 1920×1080, 30 fps and a 30-minute player lifetime.
  Playback starts paused. Offline viewing uses a cached thumbnail only.
- Background discovery has bounded capacity. Calendar integration, all mention
  and thread-notification policies, and platform-native incoming-call actions
  remain incomplete. Unknown notification eligibility stays suppressed.
- Session history retains up to 32 rooms with 100 messages per room. Eligible
  durable accounts also restore up to eight ordinary-room histories, member
  counts and deliberately staged outgoing intents from private, unencrypted
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

- The macOS app is ad-hoc signed and is not notarized. See the
  [first-launch instructions](install.md#first-launch-blocked-by-macos).
- Mac CI checks compilation, package structure, native library paths and
  signatures. Native login, permissions, devices, PDF rendering and update installation still
  need platform qualification. Windows is compile-checked only.
- Native credential storage may be unavailable or report cleanup pending.
  The explicitly selected private-file fallback is persistent but unencrypted.
- Synthetic media and successful compilation do not qualify physical devices,
  every OS, semantic accessibility or whole-process performance and energy use.

Report reproducible problems without credentials, private server details or
message contents. Security issues follow [SECURITY.md](../SECURITY.md).
