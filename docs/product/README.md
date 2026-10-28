# Product specification

These documents define the behavior of the Rust/GPUI desktop client, with macOS as the primary product platform. Requirements describe intended behavior; implemented scope and limitations are listed in [known limitations](../limitations.md) and the [feature inventory](../compatibility/feature-inventory.md). A requirement is not by itself a completed feature or performance result. HTML design artifacts are visual references.

The product is a desktop client for NextCloud Talk: one conversation contains its messages, shared files, participants, and calls. It targets **NextCloud 34.x with Talk 24.x** and **NextCloud 35.x with Talk 25.x**, subject to the [compatibility contract](../compatibility/README.md). Versions identify tested environments; capabilities, configuration, room policy, and effective permissions determine available actions.

## Canonical decisions

1. Use Rust and native GPUI. No embedded WebView, including authentication. Browser login and explicitly identified external integrations may open the system browser.
2. Preserve the server's conversation model. A voice room, webinar, presentation, channel, or classified room is a preset or attribute of a conversation, not a fabricated top-level API room type. NextCloud user groups are invitation sources.
3. Treat public guest access and discoverability as separate settings.
4. Show **“Bereit, wenn du es bist.”** only for a voluntary conference/webinar/presentation join. An explicit headphone action in a Huddle or ordinary group chat enters audio directly; selecting its room still joins nothing. Accept an incoming direct call into the call flow immediately, without that device-prejoin interstitial. Necessary OS permission prompts, recording consent, and security checks still apply. Ringing never activates a microphone or camera. [CALL-001–004](calls.md#call-001) define the behavior and races.
5. Prefer macOS Keychain, Windows Credential Manager or qualified Linux Secret Service through [ADR-006](../architecture/README.md#accounts-and-storage). When native storage is unavailable, offer durable **unencrypted private-file** persistence, repair/retry or cancellation. Normal sign-in always persists the account; legacy temporary states are not offered as a normal sign-in choice. Use a private per-user app-data directory with POSIX `0700`/`0600` or verified owner-restricted Windows DACLs. Commit backend-qualified eligibility or account-wide suppression, preserve deliberately retained P and settle abandoned N without reactivation. Recovery never silently migrates providers. [AUTH-003–005](accounts.md#auth-003) and the [security contract](../quality/security.md) govern choices and uncertainty. No primary-password fallback, file encryption or master-password setup.
6. Recognize restrictive policies before exposing features. An unimplemented security requirement blocks the affected operation; a later roadmap phase never justifies bypassing it.
7. Do not silently turn a native feature into a browser flow. Identify browser handoffs and unsupported features honestly.

The public project is `adapt2move/nextcloud-modern-talk`, licensed EUPL-1.2. These specifications are original product requirements informed by linked primary sources; upstream implementation files are not vendored here. Documentation is English; UI copy examples are German. The complete application UI is required in English, German and Spanish (en/de/es), with device-local System/English/Deutsch/Español selection and English fallback. User-authored content is not translated. [DESK-004/010](desktop-and-accessibility.md#desk-010) define preference recovery and localization coverage; behavior must not depend on translated string matching.

## Reading order

| Document | Owns |
| --- | --- |
| [Terminology](terminology.md) | Typed identities, roles, policies, and distinctions |
| [Accounts](accounts.md) | Login, account isolation, credential storage, revocation |
| [Conversations](conversations.md) | Creation, presets, invitations, guests, discovery, organization |
| [Messaging](messaging.md) | Composition, delivery, replies, threads, search, scheduling |
| [Files and content](files-and-content.md) | Uploads, media, voice messages, polls, rich content |
| [Calls](calls.md) | Direct-call acceptance, conference prejoin, native media, consent |
| [Moderation and integrations](moderation-and-integrations.md) | Lobby, breakout rooms, federation, bots, SIP, integrations |
| [Desktop and accessibility](desktop-and-accessibility.md) | Notifications, presence, preferences, offline behavior, accessibility |
| [Journeys and states](journeys-and-states.md) | End-to-end flows and shared empty/error/loading states |
| [Feature inventory](../compatibility/feature-inventory.md) | Feature IDs, planned phases, support conditions, acceptance links |
| [Baselines and sources](../compatibility/baselines-and-sources.md) | Verified source commits, published versions, evidence limits |
| [Compatibility test matrix](../compatibility/test-matrix.md) | Required profiles and executable acceptance scenarios |

## Scope and delivery order

| Phase | Outcome | Exit condition |
| --- | --- | --- |
| P0 — Specification and bootstrap | Review these product requirements first; then define Deep Modules, ownership boundaries, typed interfaces, errors, and test seams | Independent product/compatibility/security review addressed; architecture/module/interface bootstrap reviewed before full implementation begins |
| P1 — Account and conversation foundation | Native shell, secure accounts, capability resolution, conversation navigation, core messages, files, notifications, drafts, accessible states | Relevant acceptance criteria pass on both baseline families; no simulated backend presented as real integration |
| P2 — Native calls | Direct calls, deliberate conference joins, devices, screen sharing, safe participant behavior, guest joining | Real signaling/media interoperability verified; all applicable permission, consent, lobby, E2EE, and reconnect gates enforced |
| P3 — Broad parity | Extended messaging, organization, administration, breakout and recording controls, federation | Each delivered feature passes its positive, denied, absent, and failure cases; parity inventory updated with evidence |
| P4 — Conditional extensions | Provider-dependent AI, advanced integrations, effects, additional OS clients | Feasibility, privacy, protocol, native implementation, and deployment dependencies resolved separately |

Phases are an implementation sequence, not dates or a promise that every optional backend will become available. The [inventory](../compatibility/feature-inventory.md) is the complete planning ledger. Deferred features remain listed. Safety/read-only representations of later features are required as soon as their data can appear in an earlier phase.

The [storage amendment](accounts.md#spec-amendment--amend-auth-storage-01) supersedes the earlier OS-only fallback: normal sign-in is durable, with an explicitly selected private-file fallback when native storage is unavailable. Platform qualification remains separate from the storage and application interfaces.

## Acceptance and change control

Each requirement has a stable ID (`DOM-*`, `AUTH-*`, `CONV-*`, `MSG-*`, `CONTENT-*`, `CALL-*`, `MOD-*`, `INT-*`, `DESK-*`, `STATE-*`, `JNY-*`, `COMP-*`). Inventory IDs are `F-*`; test scenarios are `CT-*`. IDs must not be reused for another behavior. Acceptance paragraphs state observable outcomes, including failure behavior; they are not assertions that tests already exist.

A feature can move from **Planned** to **Implemented** only with a linked implementation and a bounded description of what works. **Verified** additionally requires reproducible evidence naming the client revision, exact server/Talk patches, OS, capabilities, policy, and backend configuration. A mock, unit test, screenshot, or successful compilation alone cannot establish interoperability. Any scoped omission must stay visible in the inventory and release notes.

Before shipping a phase, resolve conflicting requirements in these documents, the security contract, and architecture interfaces. The native interface must expose uncertainty as loading, stale, unsupported, or denied; it must never render a guessed permission as an available action.
