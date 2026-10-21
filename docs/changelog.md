# v0.0.6 — alpha

- Complete account removal through credential retirement, cleanup and confirmed
  local record removal. Explain storage failures and offer an explicit retry.
- Reuse the same account reservation when retrying sign-in. Report an existing
  account directly and allow safe, disconnected accounts without saved
  credentials to authenticate again.
- Distinguish storage refusal, pending writes and unresolved local account state.
  Add bounded sign-in and removal diagnostics without credentials or server URLs.
- Add a compact application title bar with native window actions. Move the
  Update button there and keep it hidden during calls and call preparation.
- Keep the server form and input width stable when validation appears or is
  dismissed. Match the dismiss button to the error message and clear an obsolete
  validation error after a corrected submission.
- Scale the text caret with the font's full height and restore visible input text
  after resizing a field or replacing a longer value.

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.6/docs/install.md)
· [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.6/docs/limitations.md)
