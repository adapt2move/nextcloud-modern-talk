# Current implementation and limits

The native client connects accounts, conversations, messages, files, settings,
background activity and audio/video calls. The [installation guide](install.md)
and [known limitations](limitations.md) describe the installable profile; the [feature inventory](compatibility/feature-inventory.md)
separates implemented paths from remaining parity work.

## Connected application paths

- System-browser login uses NextCloud Login Flow v2. Accounts persist through
  native storage or an explicitly selected unencrypted private-file fallback.
  Restoration, logout suppression and cleanup share one account owner.
- Compact grouped navigation includes a dedicated favorites group and active/archive views.
  Header actions open members and conversation settings; the title opens shared
  content. Messaging includes history, replies, edits/deletes, reactions, threads,
  pins, reminders, schedules, room search and explicit file upload/share/download.
  The main search includes the local-user directory. Scoped user avatars, room
  avatars and emoji icons supplement names; the footer identifies the current user.
- Shared content includes bounded image and document thumbnails, enlarged image
  previews, explicit Cloud web handoff and byte-based download progress. Published
  polls support creation, voting, permitted results and closure from chat and
  conversation settings. Server poll drafts and native PDF/office viewers remain
  separate unfinished work.
- Account scheduling reconciles room lists, nonselected-room messages and
  incoming calls. Authenticated live events invalidate observations; current
  server reads remain authoritative. Startup baselines do not replay old alerts
  or clear unread state, and unknown notification eligibility stays suppressed.
- Local appearance, English/German/Spanish, startup, notifications and device
  settings persist with rollback and explicit Retry. Account privacy and
  attachment settings use admitted protocol requests. Permission guidance and
  device checks never implicitly join a call.
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
  control/dialog transitions are integrated. The chat header uses renderer-backed
  backdrop blur on Metal/Linux Blade; Windows uses an opaque white header with
  dark controls. Newly appended messages animate once and follow the
  end after a short delay only when the reader was already there. Reduce Motion,
  including an unknown observation, suppresses decorative animation without
  delaying input or focus.

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
screen/system-audio sharing in either direction, durable offline content and
outgoing-message replay remain absent. Typing, provider translation/summaries,
voice messages, video/native document viewers, account-wide content search and parts of moderation
and integrations remain incomplete.

The pinned Rust engine supports UDP TURN but not TCP/TLS TURN or live credential
rotation. New peers use fresh settings; unsupported or unusable required
transports fail explicitly. Functional media and bounded application queues do
not establish the full allocation, memory, energy or performance targets.

Calendar and complete mention/thread notification coverage remain unfinished.
Platform-native incoming-call actions, semantic accessibility, physical-device
quality and all-platform installation need further qualification. Historical
execution reports stay private; contributor documentation describes current
behavior and reproducible development workflows.
