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
  initial loads; expose offline, denied, unsupported and uncertain outcomes honestly,
  at the smallest scope that explains them, with one calm notice style.
- Keep top-level navigation to search and one Activity destination; the conversation list
  is the main content and secondary choices sit behind its two menus.
- Respect Reduce Motion and maintain keyboard access, visible focus, readable contrast
  and text scaling. Styling alone does not establish semantic accessibility.
- UI strings are localized; user-authored messages are not. Product behavior must not
  depend on translated string matching.

[Tokens](tokens.md) records accessibility targets and palette roles.
[Shell navigation and status](shell-navigation-and-status.md) explains the sidebar structure,
the title-bar connection status and the shared notice and error system.
[Conversation dialogs](conversation-dialogs.md) describes members, bots and room settings.
[Dependency provenance](native-ui/dependencies.md) and [project notices](../../NOTICE.md)
cover the native toolkit and assets.

## Message rows

- Calm by default, detail on demand. A row shows author, time and text. The hover
  toolbar, earlier receipts and the unpin action appear only for the hovered or
  keyboard-focused row, and never move its text.
- The toolbar straddles the row's top edge at the content's right edge, identically in the
  timeline and the thread pane. The hovered row is tinted with `surface.subtle`, so the
  controls visibly belong to it. It fades in over 140 ms; Reduce Motion shows it at once.
- A send feels instant: the editor empties on admission and the message looks sent.
  Sending and waiting show only the small clock where the receipt ticks go. Only the
  unclear-status explanation (warning) and "Nicht gesendet" (error) say so in words under
  the message in 12 pt text, with plain-text actions on a neutral hover. Every
  own message shows its receipt at the bottom right of the row. A pinned message carries "Angeheftet" under its text.
- A menu belongs to the surface that opened it: its open state, anchor and visibility are
  judged against that surface's own scroll region.

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
