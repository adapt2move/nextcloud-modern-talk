# Changelog

Changes per release, newest first. Generated from Conventional Commits.

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
