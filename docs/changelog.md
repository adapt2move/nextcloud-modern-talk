# v0.0.11 alpha

- Keep ordinary user avatars decoded across chat-row changes. Retain eligible
  avatar bytes and self-profile metadata locally for seven days, revalidate in
  the background after four hours and keep the previous image on network errors.
  Unchanged images keep their renderer allocation.
- Move download progress, cancellation and completion into the application
  header. Keep the download button separate from the file's drag surface and
  require deliberate pointer movement before starting a native file drag.
- Show each account once while restoring saved conversations. Display the
  automatic connection attempt and retain an explicit retry after failure.
- Request and check macOS microphone, camera, screen and notification permission
  in the running application. Keep callback admission bounded across cancellation
  and never interpret an opened System Settings page as permission.
- Show outgoing files and images immediately in the chat, with subtle upload
  progress below the file. Clear the admitted caption and allow further text
  while uploading. Reconcile missing share acknowledgements using exact message
  references; keep the local preview until the server thumbnail arrives.
- Require a persistent macOS signing identity for publication. Support an internal
  alpha certificate pinned to the app identifier as well as Developer ID with
  team verification. Sign bundled code consistently and remove temporary CI
  keychains after building. Internal signatures provide no Apple publisher trust
  or notarization; switching from an older ad-hoc app can require new OS approval.
- Keep the light/dark badge inside the selected appearance swatch's ring.

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.11/docs/install.md)
· [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.11/docs/limitations.md)
