# Typed terminology and domain rules

Status: **Planned**. The following are semantic types for the architecture bootstrap, not implemented Rust definitions. Protocol facts are pinned in [sources S03–S08](../compatibility/baselines-and-sources.md#protocol-sources).

Our own UI and prose use **NextCloud** and business, team and workspace vocabulary across English, German and Spanish; Talk is not described as a home, Zuhause or hogar. Preserve exact technical identifiers, URLs, domains, repository paths, protocol values and verbatim source quotations. This is our product-copy convention, not a claim about upstream trademark spelling.

| Concept | Required meaning and identity |
| --- | --- |
| `ServerOrigin` | Validated HTTPS origin plus the installation's base path; not merely a hostname. Redirects do not grant permission to forward credentials. |
| `AccountId` | Opaque local identity for an authenticated account. It partitions credentials, cached content, notifications, drafts, and requests. |
| `LoginName` / `UserId` | Authentication identifier and server user identifier are distinct; never derive a WebDAV path from the display name or assumed login name. |
| `ConversationKey` | `(AccountId, ConversationToken)` for accounts; an isolated guest-session namespace plus token for guests. A token alone is never globally unique. |
| Conversation / room | The same Talk resource. Contains chat, files, participants, calls, and policy. “Chat”, “Dateien”, and “Anruf” are views or activities within it. |
| `ConversationType` | Known server values: 1 direct, 2 group, 3 public, 4 changelog, 5 former direct, 6 note-to-self. Preserve an unknown integer explicitly. |
| `ConversationAttributes` | Raw bitmask plus understood flags. At the reviewed baselines: voice room; additionally preserved/classified/channel/announcement on Talk 25. Do not discard unknown bits. |
| `ObjectAssociation` | Opaque `objectType` and `objectId`, including file, event, breakout parent (`room`), phone, or external call. These do not replace conversation identity. |
| `PresetId` | Server-provided creation configuration, including `default` and `forced`. A name is not a privilege or a security guarantee. |
| Guest access | Whether non-account participants may enter through a public conversation flow; password and admission policies still apply. |
| Discoverability | Independent `listable` scope: participants, eligible regular users, or everyone. A public link need not be searchable. |
| Invitation source | User, NextCloud group, circle/team where supported, email, or federated identity. A directory group is not a Talk group conversation. |
| `ActorKey` | Actor type plus opaque actor identifier, scoped to its server/conversation. Users, guests, bots, bridged users, and federated users must not collide. |
| Participant / attendee | Membership and role in a conversation, distinct from a connected session. One participant may have several sessions/devices. |
| `ParticipantRole` | Owner, moderator, user, guest, user following a public link, guest moderator, or unknown. A role label does not replace effective action permissions. |
| `SessionId` | Server-issued transient session identity for conversation/signaling participation. It is neither an app password nor proof that media is connected. |
| `MessageKey` | Conversation identity plus server message ID. Replies and threads retain their own parent/root relationships. |
| `ClientReferenceId` | Correlates a pending send with a server message; it is not assumed to be a server-enforced idempotency key. |
| `CallAttemptId` | Local identity for one join/accept attempt, bound to the account, room, observed call, and generation. Invalidates late callbacks on cancel/restart. |
| Direct call | An explicit incoming/outgoing call interaction in a direct conversation. Ringing is an invitation to accept, not a conference prejoin. |
| Conference | Product term for a multi-participant call in a conversation. Not a new server room type. |
| Voice room | A conversation attribute/preset with call-oriented entry. Browsing its row or background restoration must not start capture. |
| Sensitive / classified / E2EE | Separate concepts: presentation/privacy treatment; restrictive conversation policy; cryptographic call protection. None proves either of the others. |
| Preserved / archived / favorite / important | Separate concepts: server deletion restrictions; personal organization; personal shortcut; server-supported importance marker. None implies backup or indefinite retention. |
| Capability | Evidence an API behavior is available in its specified scope. It does not grant permission or ensure the required backend is healthy. |
| Effective action availability | One of `Unknown`, `Available`, `Unavailable(reason)`, `Denied(reason)`, or `TemporarilyUnavailable(reason)` after resolving all gates. |

## DOM-001

**Preserve identity and unknown data.** Given two accounts with the same room token or two actor types with the same ID, their content and state remain distinct. Given an unknown room type, flag, role, or rich object, preserve its raw value and show an honest fallback. Do not coerce it to a normal group/user/message, clear unknown flags on update, or enable policy-sensitive operations until their safety is known.

## DOM-002

**Keep independent policies independent.** Given a public, unlisted room, show guest access enabled and discovery disabled. Given a listed non-public group, do not show a guest link. Changing a preset or one policy must not silently change unrelated settings; forced server values are shown before submission.

## DOM-003

**Separate presence, participation, and media.** A selected conversation is not necessarily a joined session; a joined conversation is not necessarily an active call; an active call does not imply audio/video publication. Joining or leaving a call must not accidentally leave the conversation permanently. Show unknown/stale presence as unknown/stale, not offline.

## DOM-004

**Resolve permissions per action.** Start call, join call, bypass lobby, publish microphone, publish camera, share screen, send chat/share content, and react are separate actions. Preserve custom/default permission semantics and server policy. When `react-permission` is available, permission 256 governs reactions independently of chat permission 128. Never derive permission to publish from permission to join.
