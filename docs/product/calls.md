# Calls

Call entry and controls stay inside the conversation workspace. Every supported path follows [security Q.SEC-10](../quality/security.md#required-bootstrap-policy-decision) and the [compatibility gates](../compatibility/README.md); see [known limitations](../limitations.md#servers-and-calls) for the actual media profile.

## CALL-001

Incoming direct calls ring without capture or focus theft. Explicit Accept enters the connecting call surface, defaulting to audio/camera off and respecting start-without-media. It does not open conference device prejoin. OS, consent and security gates still apply. Native notification actions require a live current-call dispatcher and the same validation as in-app acceptance; stale banners cannot
join.

## CALL-002

Request only permissions needed by the explicit action. Denied/busy/missing devices show truthful recovery; receive-only is offered only when actually supported. Audio accept never prompts for camera. Settings handoff is not a grant; recheck permission and current call before use, without repeated reconnect prompts.

## CALL-003

Bind invitations and attempts to account, room, observed call and generation. Cancel/end invalidates pending OS/consent callbacks; duplicate accepts share one attempt. Reconcile crossed accept/cancel and leave an obsolete successful join without publication. Never intentionally restart an ended call through accept. Existing other calls require an explicit choice. Protocol atomicity remains an
interoperability obligation in [CT-012](../compatibility/test-matrix.md#ct-012).

## CALL-004

Voluntary conference/webinar/presentation join opens device prejoin (“Bereit, wenn du es bist.” in German), followed by fresh gates and final join. Preview requires explicit action and OS permission; cancel stops it. Room selection, restore, ringing and links never join a call or start capture.

## CALL-005

Deliberate direct calling and the Huddle/ordinary-group headphone action enter the audio flow without generic prejoin, preserving mute intent and all gates. Start, join and each publication permission are independent. Real Channel rooms prohibit calls for every role; a local category name changes no server semantics.

## CALL-006

Keep mute, camera, devices, participants, layouts, chat, connection state and leave reachable, with preview and transmission distinct. Moderator actions cannot turn on another user's devices. Require fresh scoped membership, permission, consent, encryption and grid evidence during calls, including changes without hash updates. Failed/partial reads, heartbeats and RTP cannot renew grants. A membership or change event that names no restriction starts a refresh at once and leaves publication on its current grant, which still ends at its own expiry; a known restriction, a forced mute or lost control connectivity suspends at once. Expiry
suspends affected publication/subscriptions; unresolved membership/consent/encryption requires local leave. Enforcement claims require reviewed numeric freshness/stop/release bounds.

## CALL-007

Device loss stops the affected stream. A device the user selects during a call replaces the current one and keeps that medium's on/off state; a previously failed medium is tried again with the new device. With the system default selected, the call follows a default that changes, for example when headphones are unplugged. A specifically selected device that is lost stops its stream and is never replaced by another device silently. Expose audio processing, output routing or camera effects only when implemented; missing blur must not be represented by a decorative toggle or leak unintended video.

## CALL-008

Screen publication needs explicit share, source selection, OS consent and publish permission, with persistent indicator and Stop. Cancel, source loss, authority loss or call end stops capture. Never migrate sources or restart a share on recovery. The initial profile is one VP8 screen up to 1920×1080/10 fps, separate from camera and audio. System audio is offered only on a qualified path;
portal/picker consent is session-specific where applicable. Permission checks/onboarding never capture.

## CALL-009

Require explicit initially unselected recording consent wherever instance/room policy requires it. An ordinary fresh but unknown consent mode conservatively requires consent; failed/stale policy grants nothing. Verified classified rooms prohibit recording and, under the established compatible contract, skip the inapplicable consent prompt even with global-required consent. Never infer that
exception from a name/version or fabricate assent after an unexpected refusal. Consent is scoped to account/room/call generation and policy; changed consent closes the old media owner and requires explicit rejoin. Unknown status is not “not recording”.

## CALL-010

Server recording controls require current moderator authority, recording capability, backend and room policy. Show actual starting/active/stopping/failed/unknown status; a timeout cannot mean stopped. Classified rooms never record. No undisclosed local fallback; audio-only and resulting-file workflows require their own implementation.

## CALL-011

Call E2EE follows the Talk web client's protocol: Olm key sessions over call signaling, one sender key per participant, AES-GCM frame encryption with a clear codec header, rotation when somebody leaves and a ratchet when somebody joins. Required or unknown required protection starts the key exchange with the call; if it cannot start, the call ends and media is never sent unencrypted. Frames that cannot be opened are dropped. The lock in the call header reports only what this client does; it is not inferred from a capability or a server flag. Transport encryption, classified privacy and chat/file storage are distinct. Interoperability beyond the checks in [limitations](../limitations.md) is not established.

## CALL-012

Hands, speaking indicators and call reactions use actual supported signaling and reaction lists. Reconcile lost/reordered updates and retire stale indicators. Layouts enforce grid policy without enabling capture and keep participant/share focus and leave available by keyboard and assistive technology.

## CALL-013

Live captions/transcription/translation remain unavailable without a complete native provider/signaling path. If offered, disclose processing and required consent, label generated output and distinguish hidden captions from stopped server processing. No implicit local capture or client-selected external provider fallback.

## CALL-014

Bound recovery and cancel on control loss even while RTP flows. Expiry and account revocation stop local media independently of remote leave. Wake checks elapsed expiry before use. Recovery needs fresh call-existence, membership, consent, security and device evidence plus valid continuity/user intent. Retain mute/camera-off; retire screen selection. Account reauthorization or unverified continuity
requires deliberate rejoin. A call whose control evidence lapsed or whose control request failed is admitted again without the user asking, at most four times within two minutes and only while the call still exists; after that the user decides. Never resurrect ended calls or transfer old callbacks/consent; ordinary leave preserves conversation membership. End-for-everyone is separately authorized and confirmed.

## Moderation and conditional call tools

- **MOD-001:** Lobby is a waiting/admission state. Publish no unauthorized conference media; timed admission follows server state and fresh effective policy, not role guesses or a local clock alone.
- **MOD-002:** Gate mute, removal, bans, roles and publish restrictions separately; never remotely unmute. Re-ring needs effective START_CALL, current active-call participation and an eligible absent attendee, not moderator status alone. Success may be a no-op and proves no delivered ring; uncertain requests never loop automatically.
- **MOD-003:** Breakouts retain parent/child identities. Configure/move/broadcast/help only through supported gates; transitions validate destination consent and permissions, avoid duplicate publication and never retarget drafts. Retain confirmed assignments on failure.
- **MOD-004:** Attendance export requires actual authority and excludes classified/hidden data. Keep CSV invite import separate; validate destinations and encode cells safely. Exports do not prove attention.
- **INT-003:** SIP is conditional on actual configuration and authority; PINs are sensitive. Validate dial-out destination, reconcile uncertainty without redialing and never infer E2EE. Classified rooms prohibit SIP.
