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
- Require a persistent Developer ID Application signature for publication. Sign
  bundled code and the app with the same team, verify that identity and remove
  the temporary CI keychain after the build. Non-publishing test packages remain
  ad-hoc signed; notarization is not implemented. Moving from an older ad-hoc app
  can require one new OS approval.
- Keep the light/dark badge inside the selected appearance swatch's ring.

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.11/docs/install.md)
· [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.11/docs/limitations.md)
