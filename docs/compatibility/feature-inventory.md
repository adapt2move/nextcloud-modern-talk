# Feature parity inventory

Every entry is **Planned**. No native integration or passing runtime test is implied. This ledger includes advanced, conditional, and externally owned functionality so that deferred scope remains visible. Requirements and acceptance are linked in the last column. Phase definitions are in [product scope](../product/README.md#scope-and-delivery-order).

Support notation: **B** = the reviewed Talk 24/25 source families document the prerequisite; **25** = additional behavior found in the reviewed Talk 25 baseline; **C** = client product requirement, not an upstream capability promise; **P** = provider/platform/native feasibility must be established; **S** = server-owned responsibility, with a client boundary rather than a replacement implementation. These are evidence categories, not assertions that a deployed server enables a feature. Exact versions and source IDs are in [baselines and sources](baselines-and-sources.md).

All server actions additionally require [COMP-001–007](README.md#comp-001): correct local/host scope, implemented API contract, current configuration, permissions, room policy, session, backend, and OS authorization. An omitted gate below is not permission to bypass it. “Safety P1/P2” means recognizing/enforcing policy before the later control UI ships.

## Accounts and identity

Evidence: S02, S17; credential requirements are the project's [security contract](../quality/security.md).

| ID | Feature | Status | Support conditions / disposition | Phase | Specification and acceptance |
| --- | --- | --- | --- | --- | --- |
| F-001 | Server connection, subpaths, TLS | Planned | C; validated installation URL and trust | P1 | [AUTH-001](../product/accounts.md#auth-001) |
| F-002 | Browser login, 2FA/SSO | Planned | B; Login Flow v2 and server/browser auth; no WebView | P1 | [AUTH-002](../product/accounts.md#auth-002) |
| F-003 | macOS Keychain persistence | Planned | C; confirmed write plus committed account/generation restoration eligibility | P1 | [AUTH-003](../product/accounts.md#auth-003), [CT-003](test-matrix.md#ct-003) |
| F-004 | Locked/missing/denied store fallback | Planned | C; explicit session-only choice, durable all-generation restore suppression, truthful uncertain-write cleanup; retain older item | P1 | [AUTH-004](../product/accounts.md#auth-004), [AUTH-005](../product/accounts.md#auth-005), [CT-003](test-matrix.md#ct-003) |
| F-005 | Additional OS vaults | Planned | P; qualified Credential Manager/Secret Service adapters; no plaintext substitute | P4 | [AUTH-003](../product/accounts.md#auth-003) |
| F-006 | Multiple accounts | Planned | C; isolated identity, caches, events, credentials | P1 | [AUTH-006](../product/accounts.md#auth-006) |
| F-007 | Guest/session identity | Planned | B; public guest access, `guest-signaling`, session/password/admission policy | P2 | [AUTH-006](../product/accounts.md#auth-006), [CONV-005](../product/conversations.md#conv-005) |
| F-008 | Expired/revoked authentication | Planned | B/C; confirmed account loss invalidates generation and stops local work/media; narrow action/session errors separately | P1 | [AUTH-007](../product/accounts.md#auth-007), [CT-002](test-matrix.md#ct-002), [CT-015](test-matrix.md#ct-015) |
| F-009 | Logout, device revocation, removal | Planned | B/C; truthful remote revocation and local cleanup | P1 | [AUTH-008](../product/accounts.md#auth-008) |
| F-010 | Protected content/draft cache | Planned | C; separate cache policy and key access | P1 | [AUTH-009](../product/accounts.md#auth-009) |
| F-011 | Manual app-password entry | Planned | C; optional explicit alternative, never primary-password fallback | P3 | [AUTH-002](../product/accounts.md#auth-002) |
| F-012 | Encrypted-file credential fallback | Planned | P; proposal only, separately reviewed opt-in; excluded from default fallback | P4 | [AUTH-004](../product/accounts.md#auth-004) |

## Conversations and organization

Evidence: S03–S05, S08, S15, S20–S21.

| ID | Feature | Status | Support conditions / disposition | Phase | Specification and acceptance |
| --- | --- | --- | --- | --- | --- |
| F-020 | Conversation list, activity, unread/call state | Planned | B; `last-room-activity`, scoped room data and permission freshness | P1 | [CONV-001](../product/conversations.md#conv-001) |
| F-021 | Direct conversations | Planned | B; `locked-one-to-one-rooms` and server identity | P1 | [CONV-002](../product/conversations.md#conv-002) |
| F-022 | Group conversations and group invitations | Planned | B; `can-create`, `empty-group-room`, `invite-groups-and-mails` as applicable | P1 | [CONV-002](../product/conversations.md#conv-002), [CONV-007](../product/conversations.md#conv-007) |
| F-023 | Public rooms, link access, passwords | Planned | B; `conversation-creation-password`, password/guest policy; no passwordless intermediate publish | P1/P2 | [CONV-005](../product/conversations.md#conv-005) |
| F-024 | Note-to-self | Planned | B; local `note-to-self`; no fabricated call participant | P1 | [CONV-002](../product/conversations.md#conv-002) |
| F-025 | Former direct, changelog, unknown room types | Planned | B/C; typed safe rendering, unknown values retained | P1 | [DOM-001](../product/terminology.md#dom-001), [CONV-001](../product/conversations.md#conv-001) |
| F-026 | Discoverable rooms | Planned | B; local `listable-rooms` and returned listable scope | P1 | [CONV-005](../product/conversations.md#conv-005), [CONV-006](../product/conversations.md#conv-006) |
| F-027 | Default/forced server presets | Planned | B; `conversation-presets`, `conversation-creation-all`; parameter merge then server invariants, reconcile returned room | P1 | [CONV-003](../product/conversations.md#conv-003), [CT-005](test-matrix.md#ct-005) |
| F-028 | Webinar/presentation presets | Planned | B; returned editable join/chat/reaction defaults, separate start/publish/lobby/consent gates | P1 safety / P3 controls | [CONV-004](../product/conversations.md#conv-004), [CT-005](test-matrix.md#ct-005) |
| F-029 | Voice rooms | Planned | B; returned preset/attribute, explicit join, start-call policy | P2 | [CONV-004](../product/conversations.md#conv-004), [CALL-004](../product/calls.md#call-004) |
| F-030 | Channels and announcements | Planned | 25; `announcement-preset`; no calls, moderator-only member list; announcement admin creation, member leave denial, all-message default | P1 safety / P3 controls | [CONV-003](../product/conversations.md#conv-003), [CONV-004](../product/conversations.md#conv-004), [CT-005](test-matrix.md#ct-005) |
| F-031 | Classified conversations | Planned | 25/C; `classified-conversations`; private/unlisted/no-SIP/no-recording, exposure/sensitive/retention invariants; stricter native cache/export policy | P1 safety / P3 controls | [CONV-004](../product/conversations.md#conv-004), [CALL-009](../product/calls.md#call-009), [CT-005](test-matrix.md#ct-005), [CT-027](test-matrix.md#ct-027) |
| F-032 | Preserved conversations | Planned | 25; `preserve-conversation`, owner toggle; API deletion/history/guest/listability locks, no exemption from classified retention | P1 safety / P3 controls | [CONV-010](../product/conversations.md#conv-010), [CT-005](test-matrix.md#ct-005), [CT-027](test-matrix.md#ct-027) |
| F-033 | Participant and guest identity/display names | Planned | B; participant API, `guest-display-names`, list restrictions | P1/P2 | [CONV-007](../product/conversations.md#conv-007) |
| F-034 | Circles/teams and email invitations | Planned | B; `circles-support`, `invite-groups-and-mails`, actual directory sources | P3 | [CONV-007](../product/conversations.md#conv-007) |
| F-035 | Email CSV import and resend | Planned | B; `email-csv-import`, specific resend endpoint and moderator authority | P3 | [CONV-007](../product/conversations.md#conv-007) |
| F-036 | Owner/moderator/guest moderator roles | Planned | B; actual role/effective permissions; 25 owner promotion gate separately | P1 safety / P3 controls | [CONV-008](../product/conversations.md#conv-008) |
| F-037 | Per-participant/default publishing/chat/reaction permissions | Planned | B; `conversation-permissions`, `publishing-permissions`, `chat-permission`, `react-permission` | P1 safety / P3 controls | [DOM-004](../product/terminology.md#dom-004), [CONV-008](../product/conversations.md#conv-008) |
| F-038 | Name/description/avatar/read-only/mention settings | Planned | B; relevant config/limits, `room-description`, `avatar`, `read-only-rooms`, `mention-permissions` | P1 display / P3 controls | [CONV-010](../product/conversations.md#conv-010) |
| F-039 | Favorites, mark unread, importance | Planned | B; local `favorites`, `important-conversations`; `chat-unread` | P1 favorites/unread / P3 importance | [CONV-009](../product/conversations.md#conv-009), [MSG-010](../product/messaging.md#msg-010) |
| F-040 | Archive and automatic unarchive preferences | Planned | B `archived-conversations-v2`; 25 optional `unarchive` setting | P3 | [CONV-009](../product/conversations.md#conv-009) |
| F-041 | Personal conversation tags | Planned | B `conversation-tags`; 25 optional `tags-show-unread` setting | P3 | [CONV-009](../product/conversations.md#conv-009) |
| F-042 | Sort/group/list density/chat style | Planned | B; advertised local sort-order/group-mode/list-style/chat-style settings | P1 local layout / P3 sync | [CONV-009](../product/conversations.md#conv-009), [DESK-004](../product/desktop-and-accessibility.md#desk-004) |
| F-043 | Sensitive conversation presentation | Planned | B; `sensitive-conversations`; classified enforcement independent | P1 | [CONV-004](../product/conversations.md#conv-004), [STATE-006](../product/journeys-and-states.md#state-006) |
| F-044 | Expiration, leave/delete, clear history, unbind | Planned | B; `message-expiration`, `clear-history`, `unbind-conversation`; applicable ownership/retention | P1 enforcement / P3 controls | [CONV-010](../product/conversations.md#conv-010) |
| F-045 | Extend a direct conversation | Planned | B; supported creation/object contract; explicit new audience/history | P3 | [CONV-002](../product/conversations.md#conv-002) |
| F-046 | User/directory/phonebook search | Planned | B; server search and optional local `phonebook-search`; privacy permissions | P1 users / P3 phonebook | [CONV-006](../product/conversations.md#conv-006) |

## Messages and retrieval

Evidence: S03, S06, S09, S12, S15.

| ID | Feature | Status | Support conditions / disposition | Phase | Specification and acceptance |
| --- | --- | --- | --- | --- | --- |
| F-050 | History, pagination, rich/system messages | Planned | B; `chat-v2`, `system-messages`, supported object formats | P1 | [MSG-001](../product/messaging.md#msg-001) |
| F-051 | Text, emoji, Markdown, code/links | Planned | B/C; `markdown-messages`, max-length, safe native rendering | P1 | [MSG-002](../product/messaging.md#msg-002) |
| F-052 | Pending/delivery-unknown reconciliation | Planned | B/C; `chat-reference-id` correlation is not assumed idempotency | P1 | [MSG-003](../product/messaging.md#msg-003) |
| F-053 | Local drafts and offline composition | Planned | C; protected storage or visible session-only drafts | P1 | [MSG-004](../product/messaging.md#msg-004) |
| F-054 | Replies and context | Planned | B; `chat-replies`, `isReplyable`, `chat-get-context` | P1 | [MSG-005](../product/messaging.md#msg-005) |
| F-055 | Private replies | Planned | B; `private-reply`, eligible direct destination and source exposure policy | P3 | [MSG-005](../product/messaging.md#msg-005) |
| F-056 | Threads, rename, subscriptions | Planned | B; `threads`, thread routes and actual permissions | P3; safe display P1 | [MSG-006](../product/messaging.md#msg-006) |
| F-057 | Reactions and author lists | Planned | B; `reactions`, separate reaction permission where advertised | P1 | [MSG-007](../product/messaging.md#msg-007) |
| F-058 | Message edits | Planned | B; `edit-messages`, author/time/policy limits | P1 | [MSG-008](../product/messaging.md#msg-008) |
| F-059 | Note-to-self extended edits | Planned | B; `edit-messages-note-to-self` | P1 | [MSG-008](../product/messaging.md#msg-008) |
| F-060 | Delete messages and shared-object messages | Planned | B; `delete-messages`, `delete-messages-unlimited`, `rich-object-delete` as applicable | P1 | [MSG-008](../product/messaging.md#msg-008) |
| F-061 | Mentions/autocomplete/direct mention badges | Planned | B; mention APIs, `mention-flag`, `direct-mention-flag`, `mention-permissions` | P1 | [MSG-009](../product/messaging.md#msg-009) |
| F-062 | Silent send and returned silent state | Planned | B; `silent-send`, `silent-send-state`; attachment metadata separate | P3 | [MSG-009](../product/messaging.md#msg-009) |
| F-063 | Typing indicators/privacy | Planned | B; signaling and `typing-privacy`/local settings | P1 | [MSG-009](../product/messaging.md#msg-009) |
| F-064 | Read markers, explicit unread, receipt privacy | Planned | B; `chat-read-marker`, `chat-read-last`, `chat-unread`, `chat-read-status`, `chat-keep-notifications` | P1 | [MSG-010](../product/messaging.md#msg-010) |
| F-065 | Conversation/account message search | Planned | B; local `unified-search`, context retrieval, permission-filtered results | P1 | [MSG-011](../product/messaging.md#msg-011) |
| F-066 | Pin/unpin/hide pins and pin expiry | Planned | B; `pinned-messages`, actual pin limits and permissions | P3 | [MSG-012](../product/messaging.md#msg-012) |
| F-067 | Reminders and upcoming reminders | Planned | B; local `remind-me-later`, `upcoming-reminders` | P3 | [MSG-013](../product/messaging.md#msg-013) |
| F-068 | Scheduled messages, edit/cancel/list | Planned | B; local `scheduled-messages`; server owns execution | P3 | [MSG-014](../product/messaging.md#msg-014) |
| F-069 | Message translation | Planned | B/P; advertised translation/task providers and language options | P4 | [MSG-015](../product/messaging.md#msg-015) |
| F-070 | Chat summaries and unread summary threshold | Planned | B/P; `chat-summary-api`, provider/config/policy | P4 | [MSG-015](../product/messaging.md#msg-015) |
| F-071 | Copy/forward permitted content | Planned | C/P; source exposure rights; supported target send/share contract | P3 | [MSG-016](../product/messaging.md#msg-016) |

## Files and structured content

Evidence: S03, S06, S09, S12, S16, S22.

| ID | Feature | Status | Support conditions / disposition | Phase | Specification and acceptance |
| --- | --- | --- | --- | --- | --- |
| F-080 | Local file upload/paste/drag/drop | Planned | B/C; local attachments allowed/folder/quota and share permission | P1 | [CONTENT-001](../product/files-and-content.md#content-001) |
| F-081 | Per-conversation attachment folders | Planned | B; `attachments.conversation-subfolders`, safe folder-probe contract | P1 | [CONTENT-001](../product/files-and-content.md#content-001) |
| F-082 | Existing Nextcloud file sharing | Planned | B; accessible file/share API and room policy | P1 | [CONTENT-002](../product/files-and-content.md#content-002) |
| F-083 | Captions, silent/thread metadata | Planned | B; `media-caption`, `threads`, operation-specific support | P3 | [CONTENT-004](../product/files-and-content.md#content-004) |
| F-084 | Shared files/media/links browser | Planned | B; `rich-object-list-media`; federation support separately | P1 basic / P3 full | [CONTENT-003](../product/files-and-content.md#content-003) |
| F-085 | Native previews, player, downloads | Planned | C/P; supported safe codecs/formats; GIF limits; explicit external opener | P1 basic / P3 full | [CONTENT-003](../product/files-and-content.md#content-003) |
| F-086 | Voice message capture/playback | Planned | B/P; `voice-message-sharing`, microphone, native interoperable codec, upload | P3 | [CONTENT-005](../product/files-and-content.md#content-005) |
| F-087 | Poll creation/voting/results/closure | Planned | B; `talk-polls`; author/moderator Close action, reconciliation and final results; closing retains the message | P3 | [CONTENT-006](../product/files-and-content.md#content-006), [CT-010](test-matrix.md#ct-010) |
| F-088 | Poll drafts/edit/publish | Planned | B; `talk-polls-drafts`, `edit-draft-poll`, moderator rights | P3 | [CONTENT-006](../product/files-and-content.md#content-006) |
| F-089 | Rich objects and location sharing | Planned | B; `rich-object-sharing`, `geo-location-sharing`, explicit content choice | P1 read / P3 create | [CONTENT-004](../product/files-and-content.md#content-004) |
| F-090 | Smart Picker/reference providers | Planned | P; native provider/API contract; browser-only providers explicitly limited | P4 | [CONTENT-008](../product/files-and-content.md#content-008) |
| F-091 | Retention/access revocation across media | Planned | B/C; message/share policy plus protected cache handling | P1 | [CONTENT-007](../product/files-and-content.md#content-007) |

## Calls and media

Evidence: S03, S04, S07, S10, S15, S19, S21. An advertised call API does not establish native WebRTC/media compatibility.

| ID | Feature | Status | Support conditions / disposition | Phase | Specification and acceptance |
| --- | --- | --- | --- | --- | --- |
| F-100 | Incoming direct accept/decline | Planned | B/C; real notification/session reconciliation; no conference prejoin | P2 | [CALL-001](../product/calls.md#call-001), [CALL-003](../product/calls.md#call-003) |
| F-101 | OS access denied / receive-only acceptance | Planned | C/P; requested media, OS access, server receive-only support | P2 | [CALL-002](../product/calls.md#call-002) |
| F-102 | Remote cancel, accepted elsewhere, stale/duplicate events | Planned | B/C; `call-notification-state-api` where supported, generation/session checks | P2 | [CALL-003](../product/calls.md#call-003) |
| F-103 | Outgoing direct audio/video calls | Planned | B/P; `audio`/`video`, allowed start/join/publish, native backend | P2 | [CALL-005](../product/calls.md#call-005) |
| F-104 | Voluntary conference/voice-room prejoin | Planned | C; explicit join, policy, preview authorization | P2 | [CALL-004](../product/calls.md#call-004) |
| F-105 | Multi-party audio/video and call chat | Planned | B/P; `in-call-flags`, `conversation-call-flags`, native media and per-stream permissions | P2 | [CALL-006](../product/calls.md#call-006) |
| F-106 | Start-call restrictions and silent calls | Planned | B; `start-call-flag`, `canStartCall`, group/admin rules, `silent-call` | P2 | [CALL-005](../product/calls.md#call-005) |
| F-107 | Internal/external signaling, STUN/TURN/HPB | Planned | B/P; actual configured transport/backend and native interoperability | P2 | [CALL-006](../product/calls.md#call-006), [COMP-007](README.md#comp-007) |
| F-108 | Microphone/camera/output devices | Planned | C/P; OS capture/routing and native media support | P2 | [CALL-007](../product/calls.md#call-007) |
| F-109 | Echo/noise/level processing | Planned | P; measured native engine behavior; no decorative toggles | P2 | [CALL-007](../product/calls.md#call-007) |
| F-110 | Blur/virtual/custom backgrounds | Planned | B/P; native effects, predefined-background settings, upload permission | P4 | [CALL-007](../product/calls.md#call-007) |
| F-111 | Screen/window sharing and remote shares | Planned | B/P; native picker/capture, screen publish permission, `video` | P2 | [CALL-008](../product/calls.md#call-008) |
| F-112 | Share system audio | Planned | P; explicitly supported OS/media path; no universal claim | P4 | [CALL-008](../product/calls.md#call-008) |
| F-113 | Gallery/focus/fullscreen/grid/speaker layout | Planned | B/C; server grid-limit/enforcement, native rendering | P2 | [CALL-006](../product/calls.md#call-006), [CALL-012](../product/calls.md#call-012) |
| F-114 | Raise/lower hand and call reactions | Planned | B; `raise-hand`, signaling, actual supported-reactions list | P3 | [CALL-012](../product/calls.md#call-012) |
| F-115 | Recording consent/status | Planned | B; `recording-consent`, fresh effective instance/room policy, verified classified exception, current status | P2 safety | [CALL-009](../product/calls.md#call-009), [CT-013](test-matrix.md#ct-013), [CT-018](test-matrix.md#ct-018) |
| F-116 | Start/stop audio/video recording, retrieve file | Planned | B/P; `recording-v1`, configured recorder/HPB, role and policy | P3 | [CALL-010](../product/calls.md#call-010) |
| F-117 | Call E2EE | Planned | B/P; call-end-to-end-encryption/config plus proven native crypto/key lifecycle | P2 gate/support | [CALL-011](../product/calls.md#call-011) |
| F-118 | Live transcription/captions | Planned | B/P; call.live-transcription, provider/backend, native path and policy | P4; safety P2 | [CALL-013](../product/calls.md#call-013) |
| F-119 | Live translation and language preference | Planned | B/P; call.live-translation and target-language config/provider | P4 | [CALL-013](../product/calls.md#call-013) |
| F-120 | Reconnect, bandwidth degradation, session replacement | Planned | B/C/P; bounded policy expiry with live media/control loss, account-loss shutdown, intent-preserving recovery | P2 | [CALL-014](../product/calls.md#call-014), [CT-015](test-matrix.md#ct-015) |
| F-121 | Leave, end-for-everyone, duration policy | Planned | B; distinct server operations, moderator rights, max-duration config | P2 | [CALL-014](../product/calls.md#call-014) |
| F-122 | Compact/floating call window | Planned | C/P; native window/accessibility behavior; not universal OS PiP support | P4 | [DESK-001](../product/desktop-and-accessibility.md#desk-001) |

## Moderation and administration

Evidence: S03–S05, S07–S08, S10–S11, S21.

| ID | Feature | Status | Support conditions / disposition | Phase | Specification and acceptance |
| --- | --- | --- | --- | --- | --- |
| F-130 | Lobby/waiting/admission | Planned | B; `webinary-lobby`, role, ignore-lobby permission, actual server semantics | P2 participant / P3 moderator | [MOD-001](../product/moderation-and-integrations.md#mod-001) |
| F-131 | Mute/remove/publish restrictions | Planned | B; `force-mute`, participant/session authority; never remote unmute | P2 enforcement / P3 controls | [MOD-002](../product/moderation-and-integrations.md#mod-002) |
| F-132 | Ban/unban participants | Planned | B; `ban-v1`, moderator authority and identifiable target | P3 | [MOD-002](../product/moderation-and-integrations.md#mod-002) |
| F-133 | Re-ring individual participants | Planned | B; `send-call-notification`, effective START_CALL, caller active participation, eligible absent local/federated user; no moderator-only gate | P3 | [MOD-002](../product/moderation-and-integrations.md#mod-002), [CT-016](test-matrix.md#ct-016) |
| F-134 | Breakout configure/create/delete/start/stop | Planned | B/P; `breakout-rooms-v1`, call.breakout-rooms, configured backend | P3 | [MOD-003](../product/moderation-and-integrations.md#mod-003) |
| F-135 | Breakout manual/automatic/free assignments and moves | Planned | B/P; actual supported modes, room/session transitions | P3 | [MOD-003](../product/moderation-and-integrations.md#mod-003) |
| F-136 | Breakout broadcast/help requests | Planned | B/P; relevant routes/permissions, parent room scope | P3 | [MOD-003](../product/moderation-and-integrations.md#mod-003) |
| F-137 | Download call participants/attendance | Planned | B; `download-call-participants`, export and room privacy policy | P3 | [MOD-004](../product/moderation-and-integrations.md#mod-004) |
| F-138 | Promote/demote owners | Planned | 25; `promote-demote-owner`, owner authority | P3 | [CONV-008](../product/conversations.md#conv-008) |
| F-139 | Recording/mention/expiration policies | Planned | B; respective capabilities plus role; recognition required before controls | P1/P2 safety / P3 controls | [CONV-010](../product/conversations.md#conv-010) |

## Federation and integrations

Evidence: S03, S07–S09, S13–S14, S16, S19.

| ID | Feature | Status | Support conditions / disposition | Phase | Specification and acceptance |
| --- | --- | --- | --- | --- | --- |
| F-150 | Federation invite/accept/reject/chat | Planned | B; `federation-v1`, incoming/outgoing/trust policy, proxy endpoint support | P3; recognition P1 | [INT-001](../product/moderation-and-integrations.md#int-001) |
| F-151 | Federated calls | Planned | B/P; `federation-v2`, host policy, native media and encryption interoperability | P3 | [INT-001](../product/moderation-and-integrations.md#int-001), [CALL-011](../product/calls.md#call-011) |
| F-152 | Federated shared items | Planned | B; `federated-shared-items` plus underlying share/permission contract | P3 | [INT-001](../product/moderation-and-integrations.md#int-001) |
| F-153 | Bots/command output and room bot management | Planned | B; `bots-v1`, installed bots, moderator and exposure policy | P3 | [INT-002](../product/moderation-and-integrations.md#int-002) |
| F-154 | Webhooks/bot feature discovery/server installation | Planned | B/S; `bot-features-api`/shared secrets are bot/server responsibilities | P3 boundary | [INT-002](../product/moderation-and-integrations.md#int-002) |
| F-155 | SIP dial-in/PIN information | Planned | B/P; `sip-support`, `sip-support-nopin`, sip-enabled/can-enable-sip and room policy | P3 | [INT-003](../product/moderation-and-integrations.md#int-003) |
| F-156 | SIP dial-out/number validation | Planned | B/P; `sip-support-dialout`, configured bridge, allowed account/group, region | P3 | [INT-003](../product/moderation-and-integrations.md#int-003) |
| F-157 | SIP direct dial-in routing | Planned | B/S; `sip-direct-dialin`, server mappings; native displays resulting room | P3 boundary | [INT-003](../product/moderation-and-integrations.md#int-003) |
| F-158 | Schedule meetings/upcoming events/mutual calendar | Planned | B; `schedule-meeting`, `dashboard-event-rooms`, `mutual-calendar-events`; calendar permissions | P3 | [INT-004](../product/moderation-and-integrations.md#int-004) |
| F-159 | File conversations and public-share integration | Planned | B; supported file/share object associations and access | P3 | [INT-005](../product/moderation-and-integrations.md#int-005) |
| F-160 | Video verification/password-release flow | Planned | B/P; explicit verification security contract; browser fallback identified | P4 | [INT-005](../product/moderation-and-integrations.md#int-005) |
| F-161 | Matterbridge/Matrix/other bridges | Planned | B/P/S; installed provider, native/API feasibility, no native Matrix claim | P4 | [INT-006](../product/moderation-and-integrations.md#int-006) |
| F-162 | External call services | Planned | B/P; recognize external_call; iframe embedding excluded; safe browser route only | P1 recognition / P4 handoff | [INT-007](../product/moderation-and-integrations.md#int-007) |
| F-163 | Assistant/call summaries/workflow integrations | Planned | P; exact provider contract and privacy/room policy, no universal endpoint | P4 | [INT-008](../product/moderation-and-integrations.md#int-008) |

## Desktop and cross-cutting behavior

Evidence: client requirements; S03/S15 for server-backed settings. Native platform behavior requires separate verification.

| ID | Feature | Status | Support conditions / disposition | Phase | Specification and acceptance |
| --- | --- | --- | --- | --- | --- |
| F-170 | Rust/GPUI native shell and navigation | Planned | C/P; actual native window/render/input/accessibility integration | P1 | [DESK-001](../product/desktop-and-accessibility.md#desk-001) |
| F-171 | OS/in-app notifications, call ringing, deep links | Planned | B/C/P; `notification-levels`, `notification-calls`, OS permission, session activity and actual transport | P1/P2 | [DESK-002](../product/desktop-and-accessibility.md#desk-002) |
| F-172 | Presence/custom status/profile avatar | Planned | B; core status/profile APIs, `single-conversation-status`, optional `temp-user-avatar-api` | P1 display / P3 edit | [DESK-003](../product/desktop-and-accessibility.md#desk-003) |
| F-173 | Read/typing/notification privacy preferences | Planned | B/C; separately scoped server/local/OS controls | P1 | [DESK-002](../product/desktop-and-accessibility.md#desk-002), [DESK-004](../product/desktop-and-accessibility.md#desk-004) |
| F-174 | Theme, language, text size, sound preferences | Planned | C; OS conventions and explicit local/server ownership | P1 | [DESK-004](../product/desktop-and-accessibility.md#desk-004) |
| F-175 | VoiceOver, keyboard, focus, accessible announcements | Planned | C/P; tested GPUI/native accessibility tree | P1 onward | [DESK-005](../product/desktop-and-accessibility.md#desk-005) |
| F-176 | Contrast, reduced motion, scaling, narrow windows | Planned | C; actual measured native UI | P1 onward | [DESK-006](../product/desktop-and-accessibility.md#desk-006) |
| F-177 | Offline cache/drafts and reconciliation | Planned | B/C; protected cache, no silent sends, membership refresh | P1 | [DESK-007](../product/desktop-and-accessibility.md#desk-007) |
| F-178 | Sleep/wake/quit/crash/multi-device sessions | Planned | B/C/P; `session-state`, `multi-room-users`, native lifecycle, no automatic capture | P1/P2 | [DESK-008](../product/desktop-and-accessibility.md#desk-008) |
| F-179 | Redacted diagnostics/error recovery | Planned | C; opt-in reviewable export, no automatic telemetry | P1 | [DESK-009](../product/desktop-and-accessibility.md#desk-009) |
| F-180 | Timezones, Unicode, localization | Planned | C; stable IDs and explicit DST interpretation | P1 | [DESK-010](../product/desktop-and-accessibility.md#desk-010) |
| F-181 | Release information/native updater | Planned | C/P; distribution/integrity contract before updater activation | P1 information / P4 updater | [DESK-011](../product/desktop-and-accessibility.md#desk-011) |
| F-182 | Additional desktop OS clients | Planned | P; future OS profiles need independent native vault/capture/accessibility/notification qualification | P4 | [DESK-011](../product/desktop-and-accessibility.md#desk-011), [CT-003](test-matrix.md#ct-003) |
| F-183 | Loading/empty/denied/unknown/partial-success states | Planned | C; all shipping surfaces, no false success labels | P1 onward | [STATE-001–007](../product/journeys-and-states.md#state-001) |

## Protocol and externally owned boundaries

Evidence: S03–S10, S13–S16. These entries prevent server/backend mechanics from being mistaken for omitted desktop controls.

| ID | Feature | Status | Support conditions / disposition | Phase | Specification and acceptance |
| --- | --- | --- | --- | --- | --- |
| F-190 | OCS/API/session/rich-object protocol | Planned | B/C; `conversation-v4`, `signaling-v3`, `no-ping`; other endpoint versions individually verified | P1/P2 | [COMP-005](README.md#comp-005), [DOM-001–004](../product/terminology.md#dom-001) |
| F-191 | Capability/hash/backport/unknown-policy handling | Planned | B/C; features-local/config-local, local/proxy hashes plus bounded non-hash policy freshness, no major-only gates | P1 | [COMP-001–006](README.md#comp-001), [COMP-003](README.md#comp-003), [CT-004](test-matrix.md#ct-004) |
| F-192 | Maintenance/rate limit/outdated client/proxy errors | Planned | B; documented 503/429/426/406/422 semantics plus endpoint context | P1 | [STATE-004](../product/journeys-and-states.md#state-004) |
| F-193 | Recorder upload/chunking/store callbacks | Planned | B/S; recording-chunked-upload belongs to recorder; client displays resulting status/file | P3 boundary | [CALL-010](../product/calls.md#call-010) |
| F-194 | Global Talk/HPB/TURN/SIP administration | Planned | S; server administrators configure services; client diagnoses prerequisites only | P2/P3 boundary | [COMP-007](README.md#comp-007), [INT-003](../product/moderation-and-integrations.md#int-003) |
| F-195 | Server feature hints, experiments, samples | Planned | B/S/C; no automatic exposure of experimental/native-unsupported behavior | P1 recognition | [COMP-004](README.md#comp-004), [DESK-001](../product/desktop-and-accessibility.md#desk-001) |

## Coverage maintenance

Before changing a baseline or claiming a release, compare its capabilities/configuration, controllers, presets, and user manual feature areas with this ledger. Add new IDs for new behavior; retain explicitly unsupported/provider/server-owned entries. Connect an implementation and its CT/Q acceptance evidence to the row when work begins. Reclassifying a feature as browser-only, unsupported, or postponed must be a visible product decision, never removal of the row.

Historical superseded API capabilities (for example conversation v1–v3, signaling v1–v2, and the old `chat` format) are intentionally represented by F-190's supported-protocol boundary, not promised as separate current features. Server-only signaling keys, ping batching, recorder secrets, bot signing, and provisioning do not become ordinary desktop preferences. Full arbitrary embedded web-plugin/iframe parity is excluded by the native/no-WebView constraint; F-090/F-161/F-162 track the remaining feasible scope explicitly.
