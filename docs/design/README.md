# Interface design

The native Rust/GPUI interface is the design reference. The [README screenshot](../../README.md)
shows the application; [known limitations](../limitations.md) distinguishes implemented
behavior from platform qualification. Historical HTML galleries and milestone captures
are available in Git history, not maintained alongside the current interface.

## Visual rules

- Use bundled Google Sans, Lucide icons, blue or sage accents, and system/light/dark appearance.
- Keep conversation navigation, messages, shared content and calls in one workspace.
- Reuse native controls and retained input entities. Composite fields have one background
  and border; focus must be visible without shifting geometry.
- Keep confirmed content visible during refresh. Use bounded static skeletons only for
  initial loads; expose offline, denied, unsupported and uncertain outcomes honestly.
- Respect Reduce Motion and maintain keyboard access, visible focus, readable contrast
  and text scaling. Styling alone does not establish semantic accessibility.
- UI strings are localized; user-authored messages are not. Product behavior must not
  depend on translated string matching.

[Tokens](tokens.md) records accessibility targets and palette roles.
[Conversation dialogs](conversation-dialogs.md) describes members, bots and room settings.
[Dependency provenance](native-ui/dependencies.md) and [project notices](../../NOTICE.md)
cover the native toolkit and assets.

## Interaction rules

Selecting a conversation joins nothing and activates no capture. An explicit audio
join in a group or huddle goes directly into audio; voluntary conference/webinar joins
use device prejoin. Accepting an incoming direct call skips conference prejoin.
Mandatory OS permission, security and consent gates still apply. Cancellation returns
to the conversation without leaving devices active. See [call requirements](../product/calls.md).

Views consume bounded snapshots and typed intents. They retain no credentials, perform
no I/O and cannot infer permissions from a screenshot or server version. Dialogs preserve
scope, drafts and focus; stale callbacks cannot change a replacement account or room.

## Screenshots

Capture the compiled app with `tools/ui/capture.py`; use fictional development scenarios
or a disposable synthetic backend. Record the platform and whether a backend is connected.
Do not include real accounts, messages, credentials or private server addresses. Screenshot
changes must come from the native app, not an edited mockup.
