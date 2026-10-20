# Conversations, discovery, and membership

This document defines conversation, discovery and membership behavior. Use the [typed domain](terminology.md) and [compatibility gates](../compatibility/README.md); see the [feature inventory](../compatibility/feature-inventory.md) and [current state](../status.md) for implemented scope and limitations. Sources: [conversation API, constants, preset factory/controller, and participant API](../compatibility/baselines-and-sources.md#protocol-sources).

## CONV-001

**Browse one coherent conversation (P1).** Show name, avatar/fallback, unread/mention markers, last activity, muted state, and a call indicator without implying all information is current. Chat, files, people, and call controls address the same room. List loading, no conversations, no filter matches, access removed, and offline cache are distinct states. Respect server read-only/changelog/former-direct/note-to-self behavior.

Acceptance: changing views preserves the room identity and draft. Opening a list row never starts microphone/camera capture. A room removed by the server disappears from live navigation after reconciliation; stale offline content is visibly stale and follows the cache policy.

The conversation header has one compact action area. A confirmed favorite has a persistent, clickable filled star beside its title; an unselected star appears on header hover or keyboard focus. The search action also appears on header hover or keyboard focus and opens the scoped search field below the header. Clicking the title opens shared media/items in a dialog. Keep member management, call entry and conversation settings as distinct actions, without a second redundant icon strip. The Huddle/headphone action uses the same unframed icon treatment as adjacent actions.

Use authoritative user avatars and conversation images or emoji icons when available, with clear initials/icon fallbacks during loading or failure. The bottom of navigation shows the current user's profile image and name, with the organization on a second line. It does not add an organization switcher.

## CONV-002

**Create direct, group, public, and note-to-self conversations (P1).** Start a direct conversation by server user identity; reuse an existing returned direct room. Create a group independently of inviting a directory group. Offer note-to-self only when supported. Public creation requires all guest/password policies. Do not offer adding third parties to a locked direct room; where supported, explicitly create an extended conversation and keep its history separate.

Acceptance: a creation restriction (`conversations.can-create`) removes disallowed group/public actions while preserving permitted direct actions. Submission errors preserve safe form input. A timed-out creation is reconciled before retry to avoid duplicate rooms. Note-to-self does not present an incoming colleague call or fabricated participants.

## CONV-003

**Apply actual server presets (P1).** Load the account's returned preset set when `conversation-presets` is available. Resolve creation in this order:

1. Merge returned default → selected preset → explicit user changes → administrator-forced parameters.
2. Apply the understood server invariants in CONV-004/010 **after** that merge. They can constrain a forced value or reject a conflicting request. Forced defaults are not permission to bypass an invariant.
3. Revalidate capability, configuration, invitation and effective-permission gates. Show the effective settings and why a field is locked, distinguishing an administrator-forced parameter from an invariant.
4. Reconcile the actual returned room, attributes, settings and invitation results before showing success or offering a link/call. Report coercions, rejected recipients and partial success; do not treat the submitted form as server state.

Do not infer a preset from a localized name or hard-code a Talk-25-only option onto Talk 24. For classified creation, the reviewed server coerces public type to private group, listability to none and SIP to disabled even after forced parameters, while rejecting nonempty object bindings. The review must show those effective restrictions, never forced-on SIP/discovery. [Pinned merge and invariant evidence](../compatibility/baselines-and-sources.md#preset-and-policy-precedence).

Acceptance: ordinary users see no administrator-only announcement preset unless actually returned and authorized. Test classified plus conflicting public/listable/SIP values, including forced parameters, and rejection of an unrelated object binding. A changed form policy causes revalidation; rejected values remain visible with the server reason. If a returned room contradicts a known restriction, block the affected actions and reconcile instead of displaying unrestricted success or retrying creation blindly. A missing preset API uses only the implemented basic creation contract and invents no unavailable presets.

## CONV-004

**Respect voice, webinar, presentation, channel, announcement, classified, and preserved behavior (recognition P1; advanced controls P3).** Present attributes as room behavior. The following rules apply when the compatible server contract is established; a version or preset label alone is insufficient. [Pinned rules and implementation](../compatibility/baselines-and-sources.md#preset-and-policy-precedence).

| Behavior | Normative rule and distinction |
| --- | --- |
| Voice room | A deliberate headphone action joins the audio flow directly under CALL-005, without generic conference prejoin; selection never auto-joins. Permission, consent, lobby and security gates remain required. The reviewed preset's regular-user listability and one-hour message expiry are editable defaults, subject to forced parameters and returned state. |
| Webinar/presentation | Reviewed ordinary-member defaults allow call join, chat and reactions, without start-call or media-publish permissions. Webinar additionally defaults to a public room and non-moderator lobby. These are preset parameters, not immutable role rules; expose actual effective permissions, admission and consent policy. |
| Channel | Calls are forbidden for every role, including owner/moderator; only participants with moderator authority receive the member list. The preset's reaction-only ordinary-member permissions and listability are defaults that can change; such edits do not remove the call/member-list invariants. |
| Announcement | Administrator-only creation; always inherits channel restrictions. Members without moderator authority cannot leave. All-message notifications are the default for participants, not a locked preference; supported personal notification changes remain possible. Moderator leave still obeys other lifecycle rules, including last-moderator restrictions. |
| Preserved | Only the owner can toggle preservation. While set, API room deletion, history clearing, public/guest-access changes and listability changes are forbidden, including for the owner until unpreserved. This does not defeat classified automatic retention. |
| Classified — access and media | Private, unlisted, no public/guest access, no SIP dial-in/out and no server recording. Sensitive treatment is forced for every participant and cannot be unset. Neither a moderator role nor forced preset parameters relax these invariants. CALL-009 defines the classified recording-consent exception. |
| Classified — invitations/exposure | Reject email guest, phone and federated invitations; local users and allowed local group/team invitation sources still follow server validation. The reviewed APIs also prohibit bot enablement, Matterbridge exposure, chat summaries, private replies quoting this room and call-participant export. Do not offer an unrelated file/event/external-call object binding; the object field is reserved for the classified retention lifecycle. This enumerates checked guards, not a guarantee against every server/plugin/external disclosure path. |
| Classified — retention | One-hour message expiry is a preset default, not an immutable rule or exact deletion deadline. Separately, an ended call queues an unbound classified room for configured automatic room retention (reviewed default one hour; zero disables that sweep). Preservation alone does not exempt it. Only the supported, explicit moderator keep/unbind transition in CONV-010 removes it from that sweep. |

**Additional native-client privacy policy:** classified content has no notification/list/search excerpt outside its active authorized room, external preview/provider processing, clipboard/forward/export action, or durable content/draft/search/thumbnail cache. Authorized in-room rendering and accessibility remain available in memory. These are deliberately stricter client rules, not guarantees supplied by a generic server permission bit. Disclose session-only drafts and unavailable exports; do not claim control over other clients, server storage or files already saved elsewhere. Sensitive alone does not imply all classified restrictions or E2EE.

Acceptance: verify channel call denial for owner/moderator/member and member-list access only for moderator authority; do not reconstruct a hidden list from caches. Verify announcement ordinary-member leave denial and editable all-message notification default. Test each classified invitation/exposure block, forced-sensitive state, unrelated object rejection, and all preserved locks. Unknown restrictive policy blocks affected operations; inability to enforce classified content handling blocks opening that content with a reason. Returning to a room after restriction/retention changes cannot revive a cached excerpt. CT-005/013/018/027 cover these distinctions.

## CONV-005

**Separate guest access, discoverability, and invitation (P1 settings; P2 guest joining).** Expose “Gäste über Link zulassen” separately from “In der Suche sichtbar”. Explain the actual audience of each discoverability scope. Password requirements are validated before a room becomes public; never briefly publish a passwordless room as a workaround. Copy links only after successful creation/update, and do not put a room password in the URL.

Acceptance: all supported guest-access/discoverability combinations render independently. A discoverable room does not imply admission or publishing rights. Password denial, expired/removed links, banned guests, and lobby admission produce separate states without leaking private membership. Revoking guest access invalidates stale invitation UI.

## CONV-006

**Find and join eligible conversations (P1).** Separate joined-conversation filtering, directory/person lookup, message search, and discoverable-room search. Optional phonebook matching (P3) requires `phonebook-search`, explicit user input, and the server's privacy contract; do not upload the user's address book automatically. Label discoverable results with known name, topic, access, and join requirements without exposing unavailable private data. Loading and zero results are not equivalent. Joining a room and joining its call remain separate choices.

Acceptance: permissions and discoverability are rechecked at join time. A result that becomes private produces “Zugriff nicht mehr verfügbar” and no stale join confirmation. Search failures retain the query and offer retry; they do not show “Keine Ergebnisse”.

## CONV-007

**Invite people and show membership accurately (P1 basic; P3 advanced).** Support individual users and permitted NextCloud groups, circles/teams, email guests, and federated identities as distinct invitation sources. Explain that importing/inviting a group is a server operation, not a promise of a permanent synchronized group binding. Show partial success and rejected recipients. Moderator email resend and CSV import are separately gated; preview and validate CSV recipients before an explicit send.

Acceptance: same-named users/groups remain distinguishable by source and server. A guest can edit their supported display name without impersonating an account identity. Invitations never auto-send during recipient search or form preview. A restricted participant list shows its policy rather than a false empty list.

## CONV-008

**Manage roles and permissions (recognition P1; controls P3).** Display actual owner/moderator/guest roles and effective per-action permissions. Promote/demote only with server support and authority. Owner transfer needs `promote-demote-owner` where applicable and explicit consequences. Editing default permissions may reset participant overrides; explain this before submission. Do not expose the obsolete call-specific permission mode as a functioning feature.

Acceptance: a permission/role change arriving while an action is pending blocks/reconciles that action. Reactions can remain permitted when chat is denied. Unknown role or permission semantics do not grant moderator powers. A denied mutation restores the last confirmed values and reports the reason.

## CONV-009

**Organize personally (P1 favorites/unread; P3 full organization).** Support favorites, archive/unarchive, important/sensitive markers, tags, sort/group modes, and list density according to the relevant local capabilities/settings. Personal archive is not room deletion. Keep user-selected grouping predictable across refresh. Talk-25-specific automatic-unarchive and collapsed-tag unread preferences appear only when advertised.

Acceptance: a failed favorite/archive/tag update rolls back visibly. A room may be archived yet have a live call or new activity; follow returned policy without losing it permanently. Sensitive presentation applies across list, search, switcher, and OS notifications. Tags on one account never organize another account's rooms.

The compact sidebar gives Inbox and Archive their own unindented top-level destinations. Only the Conversations hierarchy is indented. Its add action belongs inside the section highlight, with a small visual separation; routine reconciliation needs no redundant refresh button. Confirmed favorites appear in a separate Favorites group with filled stars. Direct conversations and Note to self precede Channels, Conferences and Huddles; other or unknown types remain explicit. Existing-conversation categories come from typed metadata, with conference presets and classified status represented separately. Names never imply room types. Rows stay single-line, with small unread counts for personal conversations and dots elsewhere.

The primary navigation search can find conversations and people. Keep scoped room filtering separate from authenticated directory results, and open or create a direct conversation only after deliberate selection of a returned person.

Custom groups support creation, rename, removal, ordering, collapse and room assignment. Supported server tags/favorites use admitted server requests and confirmed replies. Only an explicitly unsupported feature permits account-scoped local fallback; Unknown never does. Local persistence uses owner-issued durable account/room keys and bounded per-item edits, retaining unseen layout entries after refresh. Server tag handles remain scoped to the current account runtime. Failed or uncertain edits keep the last confirmed layout and offer reload; no automatic replay follows uncertainty.

## CONV-010

**Edit metadata and lifecycle settings (P3; read state P1).** Gate name, description, avatar, guest access, password, read-only mode, mention policy, expiration, and recording-consent settings individually. Show server limits. Distinguish leave room, leave current session, delete room, clear history, preserve, and keep/unbind a temporary event/phone/classified room. Destructive confirmations name the room and scope; CONV-004 invariants apply to edits as well as creation.

For a queued classified room, the supported moderator “Behalten”/unbind action changes the object association from `classified` to `classified_persist`. Confirm that returned transition before claiming exemption from the classified room-retention sweep, including future calls. It does not clear the classified attribute, sensitive treatment, exposure restrictions or message-expiry policy. Do not implement “keep” by erasing attributes/object data locally. Owner-controlled preservation and moderator keep are different actions; neither is a promise of universal retention immunity. [Retention and keep evidence](../compatibility/baselines-and-sources.md#preset-and-policy-precedence).

The [conversation-dialog design](../design/conversation-dialogs.md) places members/bots in a header-invoked dialog and room settings in General, Notifications, Privacy, Permissions and Meeting tabs. It replaces the persistent right details sidebar. Direct chats hide the member-management control. Each form exposes only confirmed capabilities and effective rights; a layout choice never grants server authority.

Acceptance: leaving a call retains room membership. A failed deletion leaves the room visible; timeout or concurrent retention during keep/delete requires reconciliation. A preserved room offers no delete/clear-history/public-private/listability mutation until an authorized unpreserve succeeds; nonowners cannot toggle it. A preserved classified room still expires under its applicable automatic policy unless the separate keep transition was confirmed. Expired messages disappear from live views and managed caches, including pins/reply excerpts; keeping the room does not restore them. No exact server cleanup time is promised.
