# v0.0.10 alpha

- Separate macOS notification permission from saved Banner/Sound choices. Add an
  explicit first request, guidance after denial and a fresh check when returning
  from System Settings. Ordinary delivery and local tests never open a prompt.
- Follow the official NextCloud Notifications v2 distinction between an empty
  list and an unavailable provider. Respect hourly retries for HTTP 204, retain
  bounded server retry hints and back off on transient failures without treating
  a failed read as a fresh notification source.
- Expire queued alerts and cancel ordinary message alerts on foreground entry,
  preventing old notices from replaying after a busy helper or permission sheet.
- Migrate direct Linux system integrations to zbus 6.2.1 and split credential and
  package-management modules by responsibility. Remove a redundant Secret Service
  connection while preserving the existing provider and saved credential format.

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.10/docs/install.md)
· [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.10/docs/limitations.md)
