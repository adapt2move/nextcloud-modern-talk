# v0.0.14 alpha

- Make calls end-to-end encrypted when the server requires it, compatible with
  the Talk web client's protocol; a lock in the call header shows when this
  client protects the call. Live interoperability is not yet verified.
- Reach TURN relays over TLS and TCP (including port 443); certificates are
  verified by the system trust store, so a company CA installed in the system
  works.
- Play audio files shared in a conversation in the app.
- Send without waiting: the editor empties at once and only the receipt clock
  shows progress. Failed sends give the words back.
- Group conversations by dragging them onto groups, create groups from the “+”
  menu, and collapse sections instantly. The list filter is now a filter icon.
- Add reactions beyond the quick grid with the system emoji picker; the message
  menu is grouped and also opens on a secondary click.
- Schedule messages with one-click times and an optional calendar.
- Record voice messages with a scrolling live trace.
- Calmer visuals: no more tile seams, smooth chat and dialog surfaces with gray
  steps, neutral hover in dialogs, a pill-shaped leave button, and screen-sharing
  access shown only as a warning when missing.

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.14/docs/install.md)
· [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.14/docs/limitations.md)
