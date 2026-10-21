# v0.0.5 — alpha

- Show attachment cards for Talk file-share messages and display the available
  thumbnail while an enlarged preview loads. Use a bounded authenticated image
  fallback when the server cannot provide a thumbnail.
- Add native first-page PDF previews on macOS when server previews are
  unavailable, with explicit Cloud and download actions.
- Reconcile sent messages without restarting the history loading state. Keep
  loaded empty conversations ready during background reads and preserve the
  reader's position and new-message feedback.
- Preserve the submitted edit through repeated Enter presses, and retain newer
  text when its earlier request completes. Ignore Enter in an empty composer
  instead of showing an unrelated error after a successful send.
- Simplify the composer to a message field, emoji, send and a compact `+` menu.
  Group reply/edit context, silent sending and scheduling into dismissible
  surfaces without discarding the draft.
- Honor sign-in cancellation even when its background save has completed and
  the app is closed immediately. Preserve the running transfer when another
  download is rejected.
- Simplify package archive assembly without changing its file layout.

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.5/docs/install.md)
· [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.5/docs/limitations.md)
