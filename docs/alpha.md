# v0.0.1 — first public alpha

NextCloud Modern Talk is a native Rust/GPUI desktop client for teams. This alpha
connects to NextCloud through the system browser and brings conversations,
messages, files, settings and ordinary audio/video calls into one native window.
It is an early release, not a replacement for every feature of the official client.

## Included

- Durable accounts with native credential storage where available, or an
  explicitly selected unencrypted private-file fallback. Restart restoration and
  logout belong to the same account owner.
- Compact grouped conversations, favorites, archive navigation, search, members
  and five conversation-settings tabs.
- Text, replies, edits, deletion, reactions, threads, message pins, reminders,
  server schedules and room search. File selection uses native dialogs; uploads
  require an explicit share, and downloads do not overwrite existing files.
- Persistent appearance, language, startup, notification and device preferences
  in English, German and Spanish. Device checks and permission guidance are
  separate from joining a call.
- A maintained local WebRTC engine using Talk signaling. Supported ordinary
  calls require current room permissions, membership and consent.
- Manual update checks against GitHub Releases, with stable/prerelease channel
  selection. Installing a newer release is explicit; the app does not replace
  its own executable in the background.

## Install

Download the matching archive and checksum file from this release. The packages
include source/provenance and third-party notices. Preserve those notices when
redistributing.

**Ubuntu 24.04 x86_64:** install the `.deb` with
`sudo apt install ./nextcloud-modern-talk-0.0.1-linux-x86_64.deb`, then launch
NextCloud Modern Talk from the application menu. A desktop session with a
registered browser and portal service is required for browser login. The tar
archive is an alternative for environments with the listed native libraries.

**Apple Silicon macOS 15.7 or later:** install the official GStreamer runtime
specified in the bundle's `Contents/Resources/INSTALL.txt`, then copy the app to
Applications. This alpha has no Developer ID signature or notarization, so
Gatekeeper may require explicit approval or block it. Do not disable system
protection. The package is compile-checked; native Mac login, permissions,
devices and installation still need user qualification.

Windows is compile-checked only; no Windows installer is included in this alpha.

## Known limits

- Tested backend baselines are NextCloud 34.0.5 / Talk 24.0.6 and NextCloud
  35.0.2 / Talk 25.0.6. Availability follows capabilities and room policy, not
  the version number alone.
- Full Talk parity is unfinished. Typing indicators, provider translation and
  summaries, voice messages, advanced file/media previews, account-wide search
  and richer forwarding are not complete. See the [feature inventory](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.1/docs/compatibility/feature-inventory.md).
- HPB/external signaling, guest/federated media, required call E2EE, screen
  sharing and system-audio sharing are not supported by this call profile.
  Receiving a remote screen offer also ends the current unsupported call;
  screen-sharing interoperability is not included. A permission-settings button
  is not a screen-sharing implementation.
- Notifications do not yet provide a complete background feed for every room,
  Calendar integration or platform-native incoming-call actions on every OS.
  Unsupported sources/actions remain unavailable.
- There is no persistent offline message cache or durable outgoing-message
  replay. An uncertain send is not silently retried.
- Physical-device, macOS/Windows runtime, semantic accessibility and complete
  memory/energy/performance qualification remain open. Synthetic devices and
  successful compilation do not establish those results.

Report reproducible problems through the repository, without credentials,
private server details or message contents. Security issues follow
[SECURITY.md](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.1/SECURITY.md).
