# Changelog

Changes per release, newest first. Generated from Conventional Commits.

## v0.0.26 alpha (2026-11-13)

### Fixes

- **media:** Stabilize Opus reception and TURN session lifetimes (#47) (`240b757`)
- **calls:** Discard malformed VP8 packets and preserve failure context (`6a7a7b1`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.26/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.26/docs/limitations.md)

## v0.0.25 alpha (2026-11-12)

### Features

- **calls:** Show the guest link among the call controls, selectable as text (`191edf9`)

### Fixes

- **threads:** Tidy the thread pane, show thread titles and give bots a name and a robot picture (`4f524b4`)
- **calls:** Let the speaking ring follow the voice and ring only the microphone in the dock (`6f8bc12`)
- **chat:** Show no card for a link the server could not read (`fab3afe`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.25/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.25/docs/limitations.md)

## v0.0.24 alpha (2026-11-12)

### Fixes

- **ui:** End a long thread title with an ellipsis before the close button (`89538e2`)
- **sidebar:** Refresh the thread list without a loading state (`f993770`)
- **chat:** Keep the timeline still when a thread gets a reply (`5e8faf5`)
- **chat:** Keep a message's quote and grouping when it is edited or gets a reaction (`42f645a`)
- **threads:** Show thread replies only in their thread (`497cf9b`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.24/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.24/docs/limitations.md)

## v0.0.23 alpha (2026-11-12)

### Fixes

- **ui:** Offer the full emoji picker for the status message (`7c6c44b`)
- **ui:** End long file names in the shared contents dialog with an ellipsis (`bfc48c5`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.23/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.23/docs/limitations.md)

## v0.0.22 alpha (2026-11-12)

### Fixes

- **calls:** Steady tile while speaking, fresher own preview, chat notice (`b17142f`)
- **calls:** Keep a call running through membership changes and recover it (`b6e5e5b`)
- **chat:** Remove the jump to unread messages (`0bd6aad`)
- **chat:** Keep polling messages while a call is running (`197c543`)
- **sidebar:** Do not announce the pending refresh of a restored list (`25a07e7`)
- **calls:** Reach signaling on networks without working IPv6 (`77465c8`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.22/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.22/docs/limitations.md)

## v0.0.21 alpha (2026-11-11)

### Fixes

- **calls:** Sign in to signaling again after an expired hello token (`a8340fc`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.21/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.21/docs/limitations.md)

## v0.0.20 alpha (2026-11-10)

### Fixes

- **calls:** Keep the call when a participant offers an unusable address (`c652eca`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.20/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.20/docs/limitations.md)

## v0.0.19 alpha (2026-11-10)

### Fixes

- **calls:** Keep the call running when the system default device changes (`a8ff461`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.19/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.19/docs/limitations.md)

## v0.0.18 alpha (2026-11-10)

### Fixes

- **ui:** Keep call video inside its tile and unify call popovers and notices (`d7dd6f6`)
- **calls:** Switch microphone, speaker and camera during a call (`db5107e`)
- **accounts:** Keep the conversation list when a refresh is cancelled (`d04fc33`)

### Performance

- **calls:** Republish the call only when its observation changed (`7926c7e`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.18/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.18/docs/limitations.md)

## v0.0.17 alpha (2026-11-10)

### Features

- **ui:** Emoji popover, frosted light card, exact dialog corners, centred onboarding (`606cf91`)
- **ui:** Continuous glass in dialogs, tidy shared content, inline emoji picker (`e13dabd`)

### Fixes

- **files:** Show shared files at once and keep them with the cached history (`c514161`)

### Performance

- **media:** Load cached thumbnails at frame cadence without re-measuring the chat (`2aace3e`)

### Other changes

- Give the picture-store test rooms distinct prefixes (`5e5335a`)
- Log panics, notification outcomes, call phases, downloads and picture timings (`c322725`)
- Keep pictures as plain files and show them from this computer first (`3d2a11e`)

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.17/docs/install.md) · [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.17/docs/limitations.md)

## v0.0.16 alpha

- Fix notifications on macOS: the notification helper now carries the app's
  identity, so macOS applies the permission you granted and banners arrive.
- Calls: your own camera shows in the call; notices float over the stage instead
  of pushing people around and say what to do; controls without permission are
  dimmed with a way to the system settings; the minimized call is calmer and rings
  your microphone while you speak; a short network hiccup no longer ends in the
  "Verbinde …" gate; the control bar and the call gates are laid out cleanly.
- Chat: delivery ticks sit at the bottom right of every own message; messages of
  one author up to five minutes apart share a header; a sent quote leaves the
  composer at once and quotes use the full width; the upload card shows clear
  progress; "Zu ungelesenen springen" goes away once you saw the unread messages
  and new messages keep following while the window is hidden.
- Previews opened in the detail view are kept between opens and restarts; your own
  just-sent picture stays visible when the server cannot serve its preview yet.
- Installing an update keeps your drafts instead of refusing the restart.
- Cleaner sidebar and window edges, readable download status, pointing-hand cursor
  on buttons.

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.16/docs/install.md)
· [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.16/docs/limitations.md)
