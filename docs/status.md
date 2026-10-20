# Current implementation and limits

The first alpha combines the native account, conversation, messaging, settings and ordinary-call paths. The [v0.0.1 prerelease](https://github.com/adapt2move/nextcloud-modern-talk/releases/tag/v0.0.1) has a deliberately limited platform and protocol scope. Installation and user-facing limits are maintained in [alpha notes](alpha.md), with individual scope in the [feature inventory](compatibility/feature-inventory.md).

## Connected application paths

- Browser login uses the system browser and NextCloud Login Flow v2. Accounts persist through native storage where available or an explicitly selected unencrypted private-file fallback. Restoration, logout suppression and exact cleanup share one account owner; unavailable native storage can leave cleanup pending rather than report deletion.
- Conversations have compact grouped navigation, favorites and active/archive views. Header actions open members and five settings tabs. Messaging includes bounded history/polling, replies, edits/deletes, reactions, threads, pins, reminders, schedules and room search. Native file selection feeds explicit upload/share and non-overwriting download.
- Local appearance, English/German/Spanish, startup, notification and device settings persist with rollback and explicit Retry. Account privacy and attachment settings use admitted protocol requests. Permission guidance and device checks do not implicitly join a call.
- The isolated [nextcloud-talk-api](../crates/nextcloud-talk-api/README.md) crate owns HTTP/OCS, protocol types and bounded Talk signaling. The application composes a maintained `libwebrtc` engine with its own meeting policy and device owners.
- Manual update checks use GitHub Releases and a stable/prerelease preference. Installation remains explicit. The CI uses Linux runtime tests and Windows/macOS compilation.

## Verification scope

The account and messaging adapters have run against disposable NextCloud 34.0.5 / Talk 24.0.6 and 35.0.2 / Talk 25.0.6 servers. Ordinary Linux desktop checks cover system-browser login, durable private-file storage, restoration, conversation actions, messaging and native file dialogs. Synthetic OS audio/video devices qualify the tested callback and retirement paths only. The ordinary account call path has exercised bidirectional VP8/Opus and SCTP on both server families, with incoming-call, reconnect and local publication-close checks. These use isolated servers and synthetic media sources; they do not establish physical-device quality or WAN/TURN behavior.

## Remaining scope

Full Talk parity is unfinished. The alpha does not implement HPB/external-signaling media, required call E2EE, guest/federated media, screen/system-audio sharing in either direction (a remote screen offer ends the unsupported call), a durable offline message cache or automatic replay of uncertain writes. Typing, provider translation/summaries, richer file previews, voice messages, account-wide search and parts of moderation/integrations remain incomplete.

The notification feed is not complete for all rooms and Calendar. Platform-native incoming-call actions and physical devices need further qualification. macOS/Windows compilation does not establish native login, permissions, devices or installation. Semantic accessibility and complete memory/energy/performance qualification remain open.

Detailed task histories and failed attempts are retained privately, not in contributor documentation. Use the [development guide](development.md) for reproducible scoped commands and the [CI policy](ci.md) for platform boundaries.
