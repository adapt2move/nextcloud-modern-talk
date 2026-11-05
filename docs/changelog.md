# v0.0.9 alpha

- Restore saved macOS accounts reliably after a restart changes the mount device
  number. Add explicit profile reassociation and clearer local-storage recovery
  messages.
- Keep read progress monotonic when scrolling through older messages. Correct
  unread indicators and historical delivery/read states.
- Display complete image previews at their natural proportions within the chat
  limits. Give loading, failure and full-screen media views a clear background
  and keep gallery controls outside the image.
- Reuse eligible local thumbnails online and keep unchanged previews ready,
  avoiding periodic downloads and reloads.
- Replace the separate upload/share screen with compact composer attachments.
  Select or drop up to eight files, add a caption and send them sequentially;
  retain individual failures and prevent uncertain writes from being replayed.
- Show ordinary files as compact filename/type/size cards with a Download action
  on hover or keyboard focus. Reserve media previews for supported formats and
  add native macOS file export by dragging to Finder.
- Keep the macOS application running and receiving notifications when its window
  closes. Explicit Quit stops the application.
- Refresh compatible dependencies, build checks and distribution notices while
  retaining the GPUI-compatible libc version.

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.9/docs/install.md)
· [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.9/docs/limitations.md)
