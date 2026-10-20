# Conversation header and dialogs

This is the design contract for conversation controls, governed by [CONV-004–010](../product/conversations.md#conv-004) and [CALL-004–005](../product/calls.md#call-004). The [interactive prototype](prototypes/conversation-dialogs.html) uses fictional data and performs no server, storage, invitation or media operation. It is a layout reference, not a native implementation or API coverage claim.

## Header

Remove the persistent right-hand conversation-details sidebar. Keep the main conversation navigation, timeline, file access and in-call chat. Preserve the reading position and draft when opening or closing a dialog.

The room title retains its personal pin/unpin action. At the right of the header, in this order:

1. **Members:** a group icon and the authorized membership count; opens the members dialog. Hide it for direct conversations and Note to self. Unknown, restricted or partially loaded membership is not a fabricated total or an online count.
2. **Call:** headphones for direct conversations, Huddles and ordinary group chats; camera for conferences/webinars and a presentation icon for presentations. Use a contextual accessible label, “Gespräch starten”, “Gespräch beitreten” or “Meeting vorbereiten”. Resolve the actual start/join permissions, current call and backend readiness before enabling it.
3. **Settings:** the rightmost gear opens the five-tab conversation-settings dialog.

A deliberate headphone action enters the audio connecting/joining flow without a generic device-prejoin page. Saved device/mute preferences, server start-without-media policy, required OS permission, consent, lobby and security checks still apply. Selecting a conversation never joins or starts capture. Conference/webinar/presentation entry retains its explicit preparation screen.

The local navigation group named “Kanäle” can contain ordinary group chats. It does not confer call authority on the server's actual Channel preset, which forbids calls for every role. Show the disabled call action with its reason. A presentation icon does not itself start screen capture or create a whiteboard.

## Members and bots

Use one centered dialog with **Personen** and **Bots** tabs. Search current members and the permitted NextCloud directory, clearly separating directory candidates from existing members. Identity, account/source and role distinguish same-named people. Search results do not send invitations.

Members have authorized role and remove actions. Removal, banning and owner transfer are distinct operations; last-owner/moderator restrictions remain visible. Member-list denial has an explanation rather than an empty list. A direct conversation cannot acquire third parties through this dialog.

CSV import belongs here: bounded local selection and parsing, recipient/source validation, duplicate and invalid-row feedback, a preview, then explicit invitation submission. Show partial failures without blindly resending successful or uncertain invitations. CSV import is client workflow over supported invitation APIs, not an assumed CSV server endpoint.

The Bots tab enables or removes already available bots for this room, subject to server authority. Bot registration and credentials remain administrative concerns. Classified restrictions prohibit bot enablement; never expose token entry or an unrestricted fallback.

## Conversation settings

Match the normal application settings: a centered dialog, stable left tab navigation, aligned labels and controls, switches for booleans, restrained separators and a scrollable content pane. At narrow widths the tabs wrap above the content. No permanent saved-status footer. A text-edit action appears when there is a draft; asynchronous failure stays beside the affected control with retry/reload appropriate to the actual outcome.

| Tab | Contents and scope |
| --- | --- |
| **Allgemein** | Name, image and description; read-only lock; personal archive; leave membership; delete the conversation for everyone. Direct-conversation identity comes from the other person's profile. Archive, leave, delete and leave-current-call must not share a destructive action. |
| **Benachrichtigungen** | Personal important flag; all messages / mentions / no messages, subject to returned capability; call notifications separately. Global banner/sound policy remains in application settings. NextCloud's important flag can override its own DND status; the operating system's Focus/DND remains respected. No critical- or time-sensitive-delivery entitlement is inferred. |
| **Datenschutz** | Sensitive/no-preview treatment, enforced across navigation/search/notifications; server message-expiry policy; clear chat history with its actual audience and consequence. Classified sensitive treatment is forced. Expiry and clear-history authority are independent of the personal sensitive preference. Preserve room-retention and preserved-room restrictions. |
| **Berechtigungen** | Effective meeting join/start and media-publishing defaults; explicit participant overrides where supported; banned people and authorized unban. Explain before a default-permission change resets overrides. Do not confuse these server rights with OS camera/microphone/screen permission. |
| **Meeting** | Moderation roles and policy, lobby, guest link/password, and breakout-room configuration/assignment. Links require confirmed guest policy and never contain passwords. Breakout rooms are meeting subdivisions, not NextCloud directory groups. Direct chats and actual Channel rooms do not expose unavailable meeting configuration. |

Guest-link policy is edited in Meeting, not duplicated as a second writable control in General. Likewise, Meeting links to membership roles rather than maintaining another role editor. Read-only content can remain visible with a reason; an unknown capability never becomes an enabled control.

## Interaction and state

- The dialog keeps the originating account/room identity. Closing it restores focus to its invoking header control. Switching the account/room invalidates pending form submissions and stale completions; it cannot silently retarget a draft.
- A confirmation names the room and whether it affects only this user, future membership, all messages, or the whole room for everyone. No destructive network action is sent on opening the confirmation.
- Preserve the last confirmed server state during loading, failure and uncertain mutation. Do not optimistically claim removal, invitation, saved access or a usable guest link. Reconcile uncertain effects before retry.
- Membership, authority, retention and classification may change while the dialog is open. Revalidate at submission; a denial does not leak a cached hidden member list or re-enable a restricted action.
- Keyboard navigation, focus return, long labels and enlarged text are required. The HTML's local interactions do not qualify native keyboard or semantic accessibility behavior.

## Server references

The pinned [Talk 25.0.6 OpenAPI](https://raw.githubusercontent.com/nextcloud/spreed/refs/tags/v25.0.6/openapi-full.json), [conversation API](https://github.com/nextcloud/spreed/blob/v25.0.6/docs/conversation.md), [participant API](https://github.com/nextcloud/spreed/blob/v25.0.6/docs/participant.md) and [chat API](https://github.com/nextcloud/spreed/blob/v25.0.6/docs/chat.md) define the applicable mutations. The [Channel preset](https://github.com/nextcloud/spreed/blob/v25.0.6/lib/RoomPresets/Channel.php) establishes its call/member-list restrictions. Actual availability still depends on each supported server family, capabilities, effective rights and room state; the prototype's controls are not a promise that all these endpoints are integrated.

Older detail-sidebar and Teamküche-prejoin sketches remain historical assets. This contract and the amended call-entry requirements supersede those interactions; no old native image is relabelled as showing the new design.
