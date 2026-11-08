# Product specification

NextCloud Modern Talk is a calm, Slack-like native Rust/GPUI workspace for teams using NextCloud Talk. Optimize daily conversation work, with few navigation points, progressive disclosure and offline-first reading and composition. Do not copy the official client's navigation or promise its entire feature set.

The shell has search, one Activity destination and a conversation list. Activity collects unread rooms, mentions, threads and reminders. Favorites and personal sections organize conversations; filters and archive live in list menus. Chat, files, people and calls belong to the same conversation. Open details and settings on demand, rather than keeping a second navigation strip or permanent details
pane.

| Specification | Owns |
| --- | --- |
| [Messaging and conversations](messaging.md) | Navigation, room policy, messages, files and integrations |
| [Calls](calls.md) | Entry, capture, consent, media and moderation |
| [Accounts](accounts.md) | Authentication, isolation and storage eligibility |
| [Desktop and accessibility](desktop-and-accessibility.md) | Notifications, preferences, lifecycle and usable interaction |

Target families are NextCloud 34 / Talk 24 and NextCloud 35 / Talk 25. The [compatibility contract](../compatibility/README.md) governs actual availability; [known limitations](../limitations.md) records delivered scope. Requirements are acceptance criteria, not evidence of implementation or native qualification. [Security](../quality/security.md) and [architecture](../architecture/README.md)
retain detailed trust, transaction, ownership and lifetime contracts. Public docs are English; application UI is English, German and Spanish. Preserve EUPL-1.2 and third-party provenance. Browser handoffs are explicit; no embedded WebView.

## Shared acceptance rules

- **DOM-001:** Scope identities to account, host and room; actor type is part of actor identity. Preserve unknown protocol values without granting authority or rewriting them as ordinary rooms/users.
- **DOM-002:** Guest access, discoverability, invitation, preservation, personal archive, sensitive treatment and E2EE are independent concepts. A preset name is neither a type nor a privilege.
- **DOM-003:** Selection, room membership, signaling session, call connection and media publication are separate states. Unknown presence is not offline.
- **DOM-004:** Resolve start, join, lobby bypass, each media publication, chat and reactions independently from effective permissions; a role label never grants all actions.
- **STATE-001:** Bound and cancel loading, queues and retries. Retain confirmed content during refresh, preserve input under backpressure and reject obsolete completions. Pending UI follows admitted work only.
- **STATE-002:** Show empty only after a successful scoped read; failed, denied and offline-without-cache are distinct.
- **STATE-003:** Distinguish unknown, unsupported by client, unavailable on server, policy-denied and temporarily unavailable. Unknown security evidence cannot enable an action.
- **STATE-004:** Preserve intent through network failure, rate limits and maintenance. Reconcile uncertain writes read-only before deliberate retry; never blindly replay a possibly applied mutation.
- **STATE-005:** Account/runtime, room and call generations fence all asynchronous work. Revocation, expiry, removal and end invalidate affected actions even during dialogs; late success cannot restore authority or content.
- **STATE-006:** Apply privacy and retention to previews, search, notifications, accessibility, caches, clipboard and external handoffs, as well as the timeline.
- **STATE-007:** A click, animation, mock or compilation is not success. Confirm only the responsible owner's observed outcome and explain partial or uncertain completion.

Requirement IDs retain their meanings for existing verification references; retired journey checklists are replaced by the area criteria and the [compatibility scenarios](../compatibility/test-matrix.md). Optional integrations below define safety when encountered, not a commitment to implement every upstream feature. Keep task history and parity reports outside the specification.
