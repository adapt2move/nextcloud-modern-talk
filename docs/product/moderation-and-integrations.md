# Moderation and conditional integrations

This document defines moderation and conditional integration behavior. The [feature inventory](../compatibility/feature-inventory.md) and [current state](../status.md) record implemented scope and limitations. Sources: [participant, webinar, breakout, bot, federation, integration, and external-service contracts](../compatibility/baselines-and-sources.md#protocol-sources). Backend availability, account authority, and host/room policy are separate conditions. Missing implementation is a visible limitation.

## MOD-001

**Lobby and admission (P2 participation; P3 management).** A participant awaiting admission sees the room, waiting state, permitted chat, and leave. No unauthorized media is published to the conference. Moderator controls expose only admission/lobby actions actually provided by the server; distinguish opening a lobby to a class of participants from individually admitting someone. Timed lobby rules follow server state, not just the client clock.

Acceptance: join refusal due to lobby is a waiting/admission state, not a generic network error. Moderator promotion, lobby opening, return-to-lobby, room deletion, and cancellation while waiting reconcile correctly. No role inference bypasses admission; a moderator's ability to ignore the lobby comes from effective policy. Rejoining re-evaluates current admission.

## MOD-002

**Moderate participants and re-ring eligible invitees (P3; enforcement P2).** Support permitted mute, remove, ban/unban, role changes, and publishing-permission changes with clear scope and identity. Distinguish remove from this call/session, remove membership, and ban future reentry. Never remotely unmute or activate another participant's camera.

Re-ring requires `send-call-notification`, enabled/eligible calls, current participation in the active call and effective `START_CALL` permission. An ordinary member satisfying these gates may re-ring; moderator status alone is neither necessary nor sufficient. Resolve the target by attendee ID in the same room: only eligible local/federated user actors absent from all call sessions qualify, subject to host/proxy support and target status. Do not present guest/group/phone targets as supported re-ring recipients. Respect returned DND/unavailable outcomes without inventing remote presence. The API prose says moderator-only, but both pinned controllers use effective `START_CALL`; [the source discrepancy](../compatibility/baselines-and-sources.md#re-ring-authorization-and-response-discrepancy) records the evidence and response limits.

Acceptance: guests/federated users with identical names remain distinguishable. Denied or obsolete moderation requests do not permanently remove a participant optimistically. A removed native participant stops capture and exits the applicable session; ban reason/list access follows privileges. For re-ring, test an allowed ordinary member, denied/revoked start permission, caller outside the call, ended call, invalid/wrong-room target, target already in call and DND/unavailable target. An already-in-call target receives no new ring; the reviewed service may return success without notification, so HTTP success never proves delivery or hearing a ring. DND/status denial stays a target-level refusal, not account revocation. Reconcile an uncertain request before a deliberate retry; no automatic ringing loop or creation of a new call.

## MOD-003

**Breakout rooms (P3).** Require `breakout-rooms-v1`, enabled configuration, compatible signaling, and appropriate role. Moderators can configure automatic/manual/free assignment where supported, create/remove rooms, start/stop breakouts, move participants, broadcast a message, and respond to help requests. Participants see their assignment, permitted room choices, assistance action, and return path. Breakout rooms retain their parent association and own tokens.

Acceptance: moving rooms never leaves duplicate microphone publication or posts a draft into the wrong room. Before a move, communicate the destination; apply its lobby/consent/security requirements. Stopping breakouts returns participants or clearly explains pending transition. A failed reorganization keeps server-confirmed assignments. Parent/end/permission changes during transition cancel obsolete work safely.

## MOD-004

**Participant exports and administration (P3).** Expose attendance download only with `download-call-participants` and applicable authority; identify scope and destination before saving. Handle email CSV invitation import separately from export. Room administration includes the settings in [CONV-010](conversations.md#conv-010), not global server administration.

Acceptance: a non-moderator cannot export hidden member/attendance data using cached lists. Classified and other restrictions override generic export availability. A canceled export does not leave a completed-looking file. User-generated CSV cells are safely encoded for downstream spreadsheet use. The application never claims an attendance export proves attention or actual listening.

## INT-001

**Federated conversations (P3; safe recognition P1).** Show remote host and federated actor identity. Support allowed invitations, pending accept/reject, and conversation participation through the documented local proxy. Respect federation enabled/incoming/outgoing/trusted-server settings. Resolve local-only features/settings separately from host capabilities. Chat requires its documented federation support; calls additionally require `federation-v2` and media compatibility.

Acceptance: unsupported proxy operations do not fall back to the wrong local room or leak credentials to a host. Remote failure is distinct from missing local permissions. Shared items require their own federation support. Declining an invitation or leaving has truthful local/remote synchronization states. Same-named remote actors are not presented as trusted local colleagues.

## INT-002

**Bots, commands, and webhooks (P3 consumption/management).** Render bot/command output as untrusted content with actor attribution. List enabled bots and let authorized moderators enable/disable installed bots through `bots-v1`. Show the room/data exposure implication before enabling a bot. Offer server-advertised slash commands with argument help when a supported discovery contract exists.

The room's Bots tab loads its list automatically on entry once connectivity and confirmed authority are available. Show in-place skeletons during the admitted first request and retain a known authorized list across tab changes and refreshes. Do not repeat a successful load on every visit or automatically retry a failed or uncertain request. Offer an explicit retry for those outcomes. Loading and changing bots are unavailable offline.

Acceptance: typing a command does not execute it until send. Bot errors remain visible and do not masquerade as user messages. Classified/restricted rooms cannot enable prohibited bots. Server-wide bot installation, webhook signing, shared secrets, and bot backend operation remain administrator/server concerns; the desktop client never needs a bot's shared secret to manage its ordinary room participation. No local shell commands run from chat.

## INT-003

**SIP dial-in and dial-out (P3).** Expose call-in information only when SIP is configured/enabled for the room and the viewer may see it. Distinguish personal attendee PIN from PIN-less policy; treat both identifiers as sensitive. Dial-out additionally requires configuration, `sip-support-dialout`, appropriate account/group authority, start-call permission, and permitted room policy. Normalize/validate numbers using server region settings; show the actual destination before call initiation.

Acceptance: a failed/busy/rejected dial-out is not a connected participant. Retry after an ambiguous request reconciles the current call ID instead of dialing twice. External phone participants are labeled, and SIP is not advertised as E2EE without verified compatibility. Classified restrictions block dial-in/out. Direct dial-in routing and bridge credentials are server administration, not a native SIP softphone feature.

## INT-004

**Calendar and event-linked conversations (P3).** Support server-scheduled meetings, upcoming event rooms, supported mutual calendar context, and links to the external Calendar application. Identify timezone and room association. Scheduling a meeting differs from scheduling a chat message or starting a call now. Temporary event/instant/phone-room retention and keep/unbind actions must be explained.

Optional local Calendar notifications default off. Enable their controls and category Test only when the admitted account has verified its actual Calendar notification integration; unknown availability, generic DAV support or a saved preference is insufficient. Retain saved choices when the source becomes unavailable, suppress delivery, and explain the unavailable source. Do not invent calendar events or local reminder timers. Turning delivery off never deletes server reminders or events.

Acceptance: saving a meeting does not start devices or a call. A failed server schedule does not create a local calendar promise. Daylight-saving transitions and account timezone differences show an unambiguous instant. Calendar permissions are respected; unavailable event details do not become a guessed “free” status. Removing a calendar association is a distinct, confirmed action.

## INT-005

**Files collaboration and video verification (P3 ordinary file conversations; P4 verification).** Open a supported file-linked conversation while preserving its `file` association. Public-share/password-verification conversations have their own security-sensitive flow; explicit human verification must precede any action that releases access. Unsupported verification presents a clearly labeled browser continuation if a safe server route exists.

Acceptance: opening a file discussion does not grant the room broader file access automatically. A successful video connection is never interpreted as identity verification or consent to reveal a password. Revoked shares and missing files have distinct unavailable states.

## INT-006

**Matterbridge and app integrations (P4).** Show bridged actor identity and that content may reach an external service. Configuration requires installed/enabled Matterbridge and an actual native/API settings contract. Keep bridge secrets in the responsible server/secret-store boundary; do not copy them into ordinary client preferences. Matrix and other bridge integrations are conditional provider features, not native protocol support merely because their names appear upstream.

Acceptance: disabled bridges, unsupported authentication/recovery workflows, and provider errors leave normal Talk behavior intact where permitted. Prohibited room policies block bridging. The UI does not claim Matrix E2EE, key backup, or provider parity without explicit implementation and tests. General web app integrations get only the supported native action or an identified external handoff.

## INT-007

**External call services (P4 handoff; recognition P1).** The reviewed Talk external-call-service contract is based on an iframe and cross-window messaging. This client must not embed it. Recognize `external_call` associations and distinguish their calls from native Talk media. A system-browser handoff is possible only after validating a documented, safe user-facing route; otherwise say the service is unsupported in this client.

Acceptance: clicking an external call never invokes the native Talk call API by guesswork. No server shared secret, iframe script, or arbitrary postMessage bridge runs in the native app. Returning from the browser does not imply attendance, recording consent, or call completion. Full arbitrary iframe-service parity is excluded by the no-WebView product constraint and remains explicitly tracked in the inventory.

## INT-008

**Provider-dependent AI and automation (P4).** Chat summaries, translation, live transcription, call summaries, assistants, and workflow-generated content depend on the respective APIs/apps/providers and data-access policy. Show who performs processing and distinguish generated results from original messages. Do not enable processing simply because an app is installed.

Acceptance: denied/disabled/failed processing preserves source content and gives a recoverable state. No client-chosen external service receives conversation content automatically. Unknown provider capabilities stay unsupported until a specific contract exists. Generated output cannot trigger privileged client actions without explicit user intent.
