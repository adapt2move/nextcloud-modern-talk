# v0.0.7 — alpha

- Keep loaded conversations and messages visible during refreshes and temporary
  outages. Distinguish loading, offline, failed and empty states with consistent
  feedback and working retry actions.
- Recover cleanly from gateway errors, temporary access failures and expired
  signaling tokens. Reject obsolete history failures after a newer successful
  read, and keep thread failures scoped to their thread.
- Retry sign-in with the original server installation address. Restore focus
  after asynchronous onboarding steps and browser return.
- Keep dialog keyboard focus inside the active surface. Prevent reply actions
  from accidentally submitting a draft as an edit to another message.
- Improve native text entry, Unicode selection, composition undo, password
  masking and wrapped-line cursor movement. Preserve committed drafts while
  an input method is composing, and repair a nullable macOS input callback.

[Installation](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.7/docs/install.md)
· [Known limitations](https://github.com/adapt2move/nextcloud-modern-talk/blob/v0.0.7/docs/limitations.md)
