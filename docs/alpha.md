# v0.0.2 — alpha

NextCloud Modern Talk is a native Rust/GPUI desktop client for teams. It connects
through the system browser and brings conversations, messages, files, settings
and audio/video calls into one window. Full official-client parity remains in
progress.

## Included

- Persistent accounts through native credential storage where available, or an
  explicitly selected unencrypted private-file fallback, with restoration and
  logout handled by the same account owner.
- Compact grouped conversations, favorites, active/archive navigation, member
  management and five conversation-settings tabs.
- Text, replies, edits, deletion, reactions, threads, pins, reminders, server
  schedules and room search. The title opens shared content; Ctrl+F or Cmd+F
  opens search. Reply and overflow actions appear on message hover or focus.
- Bounded background room/message reconciliation and incoming-call discovery
  across admitted accounts, without marking messages read or replaying old
  notifications on startup.
- Persistent appearance, language, startup, notification and device preferences
  in English, German and Spanish. Lucide icons, contextual tooltips and short
  transitions respect Reduce Motion. Device checks remain separate from calls.
- New incoming and sent messages enter gently. The view follows after a brief
  delay only when already at the end; scrolling up preserves the reading position.
- Rust WebRTC with GStreamer VP8/Opus codecs, ordinary Talk signaling and the
  qualified HPB/Janus profile. Calls require current permissions, membership and
  explicit capture intent. UDP STUN and forced UDP TURN relay have been exercised.
- Randomized daily update checks and background downloads from GitHub Releases,
  plus a manual Settings check. A verified update exposes a small Update action;
  it is hidden during calls and waits for application work to drain before
  installation and restart.

## Install and update

Download the matching package and checksum file from this release. Preserve the
included source/provenance and third-party notices when redistributing.

**Ubuntu 24.04 x86_64:** install with
`sudo apt install ./nextcloud-modern-talk-0.0.2-linux-x86_64.deb`, then launch
NextCloud Modern Talk from the application menu. Browser login needs a desktop
session with a registered browser and portal service. The tar archive is an
alternative for an owned installation with the required native libraries.
System package updates use PackageKit and may request administrator approval;
an unavailable installation route is reported in Settings.

**Apple Silicon macOS 15.7 or later:** install the official GStreamer runtime
specified in `Contents/Resources/INSTALL.txt`, then copy the app to Applications.
This alpha has no Developer ID signature or notarization. Gatekeeper may require
explicit approval or block it; do not disable system protection. The package is
compile-checked. Native Mac login, permissions, devices and bundle installation
still require platform qualification.

Windows is compile-checked only; no Windows installer is supplied.

Upgrade **v0.0.1 manually**: that release has no in-app installer. From v0.0.2,
select the prerelease channel to receive later alpha releases. Downloads are
automatic; installation remains an explicit action. Unsaved message composition
must be resolved first, and an update never ends a call implicitly.

## Known limits

- Server baselines are NextCloud 34.0.5 / Talk 24.0.6 and NextCloud 35.0.2 /
  Talk 25.0.6. HPB checks use Talk 25.0.6, signaling 2.1.1 and Janus 1.3.0.
  Availability follows capabilities, configuration and room policy.
- TURN over TCP/TLS and live TURN credential rotation are unavailable in the
  pinned engine. Relay-required calls do not fall back to host candidates.
  UDP relay tests do not establish connectivity through every enterprise network.
- Guest/federated media, required call E2EE, screen/system-audio sharing and
  conference-scale qualification remain outside this profile. A remote screen
  offer ends the unsupported call. Permission guidance is not a sharing engine.
- Typing indicators, voice messages, advanced file/media previews, account-wide
  search and parts of moderation remain incomplete. See the
  [feature inventory](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.2/docs/compatibility/feature-inventory.md).
- Background discovery has bounded capacity. Calendar integration, all mention
  and thread-notification policies, and platform-native incoming-call actions
  remain incomplete. Unknown notification eligibility stays suppressed.
- There is no persistent offline message cache or durable outgoing-message
  replay. An uncertain send is never silently retried.
- Native storage may remain unavailable or report cleanup pending. Synthetic
  media and successful compilation do not qualify physical devices, every OS,
  semantic accessibility or whole-process performance and energy targets.

Report reproducible problems without credentials, private server details or
message contents. Security issues follow [SECURITY.md](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.2/SECURITY.md).
