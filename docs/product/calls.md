# Direct calls and conferences

This document defines direct-call and conference behavior. The [feature inventory](../compatibility/feature-inventory.md) and [known limitations](../limitations.md) record implemented scope and limitations. Protocol evidence: [call, signaling, recording, constants, and capability sources](../compatibility/baselines-and-sources.md#protocol-sources). Every criterion below is a release requirement for the relevant supported path, not a report of successful tests.

## CALL-001

**Incoming direct call: accept directly (P2).** In a direct conversation, ringing shows the caller, account/server context, accept, decline, and dismiss. Default **“Annehmen”** accepts audio, with camera off. If video is supported, a separate **“Mit Video annehmen”** action can explicitly request it. Respect `start-without-media`, OS permissions, and publishing restrictions. The acceptance control communicates its media intent accessibly.

Accept changes to the in-call surface with a connecting state. It never routes through the **“Bereit, wenn du es bist.”** conference device-prejoin screen. A required OS prompt, recording-consent dialog, or security explanation may interrupt connection; these are specific requirements, not a generic device check. Ringing and dismissed notifications do not open capture devices, request their OS permissions, or publish media. Direct-call capture waits for the applicable consent/security gates. An incoming notification must not steal focus and turn an unrelated Enter/Space keystroke into acceptance.

Acceptance: for an allowed, authorized direct audio call, one accept action begins connection and publishes only the explicitly requested, permitted microphone stream after all gates pass. Camera remains off. If policy says start without media, acceptance connects without publishing and offers in-call controls. Notification acceptance and in-app acceptance behave identically and create at most one call attempt.

Incoming calls have a separate local notification policy and native presentation/action capability. Where special native call UI or actions are unsupported, show that limitation alongside any ordinary OS banner/sound fallback; successful fallback delivery does not satisfy native-call acceptance. Expose Accept/Decline only when connected to the admitted current call owner. Revalidate account runtime, conversation and current invitation at activation; an ended or replaced call cancels queued notification work and cannot be accepted from a stale banner. Conference-participation notices use important-notices policy and open the voluntary prejoin route in CALL-004, never direct-call acceptance. Ringing, notification tests and notification display confer no capture authority.

## CALL-002

**Direct-call permission and device failures (P2).** On explicit accept, request only permissions needed for the selected media. Microphone denial yields a receive-only call when the server/media path permits it, with “Mikrofon nicht verfügbar”, an in-call settings/retry action, and leave. If receive-only participation cannot work, show a blocked connection with retry/leave. Camera denial falls back to permitted audio/receive-only with a visible explanation. No camera prompt is needed for audio acceptance.

Acceptance: first-use OS prompt, prior denial, policy-managed denial, no device, and a device becoming busy are each covered. The UI never shows microphone/camera as live when capture failed. Retrying permission is deliberate; it does not repeat prompts on every reconnect. Opening System Settings never constitutes permission grant. Returning from settings rechecks both authorization and whether the call still exists.

## CALL-003

**Direct-call cancellation, duplicate events, and races (P2).** Bind each incoming invitation and accept attempt to its account, conversation, observed call state, and a local generation. Refresh stale invitations using the supported call-notification state and room/session APIs. Do not fabricate a standalone accept/decline endpoint or assume a notification ID is a universally available call-instance ID.

| Event | Required outcome |
| --- | --- |
| Caller cancels while ringing | Stop ringing, invalidate accept, show ended/missed as appropriate; no capture |
| Caller cancels during an OS prompt or consent | Invalidate the attempt; late approval cannot join or publish |
| Accept and cancel cross in flight | Reconcile server state; immediately leave any obsolete successful join, stop capture, and show ended |
| Duplicate notification/accept action | Reuse one live attempt; do not create another session/media pipeline |
| Accepted on another device | Dismiss stale ringing when confirmed; do not claim this device connected |
| Decline/dismiss with network loss | Stop local ringing; report uncertain server synchronization without claiming a global hangup |
| Old response after another incoming call | Ignore it for the new call; never transfer its consent or devices |
| User already in another call | Show explicit stay/switch/decline choices; no hidden second capture or automatic hangup |

Acceptance: once cancellation/end is observed, no late callback may publish media or auto-start a new call. Before joining an accepted invitation, revalidate the observed call; acceptance must not intentionally start a new call after cancellation. The join API can also start calls, so atomic race handling is an interoperability proof obligation ([CT-012](../compatibility/test-matrix.md#ct-012)); if the native/protocol combination cannot satisfy it, record a release blocker rather than claim race-free acceptance. A user may deliberately call back through a separate action.

## CALL-004

**Voluntary conference prejoin (P2).** An explicit conference, webinar or presentation “Beitreten”/“Konferenz starten” action opens **“Bereit, wenn du es bist.”** Show room/account, microphone, camera, selected devices, preview/test where authorized, and final join. Initial media intent honors server start-without-media and user settings; preview capture requires an explicit action and OS permission. Cancel stops preview capture and leaves the conversation available.

Acceptance: selecting a room, restoring a window, receiving a conference notification, or opening a link never starts capture or joins a call. A conference notification action opens prejoin; a direct-call acceptance does not. The final join revalidates current policy and devices. A deliberate headphone shortcut for a Huddle/voice room or an ordinary group chat follows CALL-005 directly, preserving all required gates without the generic device-prejoin screen.

## CALL-005

**Outgoing direct calls and start/join restrictions (P2).** A deliberate direct “Anrufen” action, or headphone start/join in a Huddle or ordinary group chat, starts the in-call connecting/ringing surface without a conference prejoin. Selecting a room remains navigation only. The group-chat shortcut defaults to audio intent subject to saved mute/device state and server policy; it never enables camera or screen sharing. A local sidebar “Channels” group does not override the real Channel preset's call prohibition. Clearly communicate audio/video intent. Resolve call-enabled policy, `canStartCall`, start/join/publish permissions, lobby, room attributes, active-call state, and backend readiness independently. Silent start is an explicit, separately gated conference choice.

Acceptance: a participant allowed to join but not start can join an active allowed call, but cannot start one after it ends. A server-wide start-call restriction, channel, or read-only room blocks the forbidden action even for a familiar role label. Ring timeout/cancel ends the local attempt and capture; an unanswered call is not shown as connected.

## CALL-006

**In-call controls and native media (P2).** Provide mute, camera on/off, device choice, participants, speaker/focus/gallery layouts, call chat, elapsed time, connection state, and leave. Label local preview versus transmitted media accurately. Observed restrictions invalidate affected grants at once and are enforced within the approved bound; unseen changes follow [Q.SEC-10's ongoing freshness contract](../quality/security.md#required-bootstrap-policy-decision). A moderator can stop publication but never remotely activate another user's microphone/camera. Leaving releases capture and the current call session while preserving conversation membership.

Acceptance: joining, signaling ready, and actual media connected are distinguishable. Switching accounts/windows does not lose active-call controls. Media loss displays reconnecting/degraded. During uninterrupted calls, effective permissions, consent, E2EE and grid policy require authoritative refresh even when the capability hash is unchanged. Unknown/stale policy grants no new join or publication expansion; existing grants expire and suspend affected publication/subscriptions under Q.SEC-10, even if media still flows. Numeric freshness/enforcement/device-release bounds and source mapping must be approved at `G.BOOT` before these cases can pass. Cross-client media, internal/external signaling, TURN and grid enforcement need real checks; screenshots and codec availability are insufficient.

## CALL-007

**Choose devices and process audio safely (P2; advanced effects P4).** Select microphone, camera, and supported output device; show system-default separately from a specific device. Handle unplug/replug, sleep, Bluetooth changes, and device busy states without claiming capture from a missing source. Provide tested echo cancellation, noise suppression, and level behavior where the native media engine supports them; expose limitations rather than decorative toggles.

Acceptance: device loss visibly stops the affected stream. Switching to a different physical microphone/camera requires a known user-approved selection/fallback policy and must not silently unmute. Mute remains muted across reconnects. Unsupported output routing or effects are clearly unavailable. Virtual background/blur and uploads are optional, require native processing and relevant server policy, and must fail without leaking an unintended camera stream.

## CALL-008

**Screen sharing (P2).** Require explicit share action, source picker, OS screen-capture authorization, and publish-screen permission. Distinguish entire display, window, and any actually supported application/audio-sharing options. Preview source metadata without implying hidden windows cannot appear. Show a persistent sharing indicator and stop control; changing source is deliberate.

Interactive setup automatically requests each supported permission once on entering its step, then advances only on an observed grant. Privacy retains explicit checks, supported requests and OS-settings actions with guidance. A macOS restart handoff resumes at setup completion while preserving the actual observed permission states. Chat remains usable without microphone, camera or screen grants. Checking or opening Settings never starts capture. macOS screen-capture preflight can confirm authorization; a negative result is not enough to distinguish a first request from denial. Windows capture-picker consent and Linux ScreenCast portal consent belong to the selected sharing session, not a permanent global switch. An unavailable portal or native capture adapter remains unavailable, including on X11; do not infer a grant from desktop type alone. See [Apple screen preflight](https://developer.apple.com/documentation/coregraphics/cgpreflightscreencaptureaccess()), [Windows capture picker](https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture), and [ScreenCast portal lifecycle](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html).

Acceptance: cancel/denial publishes nothing. Closing the shared window, losing permission, ending the call, or being removed stops capture. A share cannot migrate automatically to another display/window. System audio is offered only if implemented and verified for that OS/media path; microphone audio is a separate control. Incoming remote shares have native resize/fullscreen/focus behavior.

## CALL-009

**Recording consent and status (P2 enforcement; P3 controls).** Resolve fresh instance policy, room `recordingConsent`, classified state and the compatible join contract before joining:

| Established policy | Required behavior |
| --- | --- |
| Ordinary room; instance requires consent | Require explicit, initially unselected consent regardless of the room default. |
| Ordinary room; instance delegates to room policy | Require consent when the room requires it. |
| Ordinary room; instance disables the consent requirement | Do not invent required consent from the room default alone; keep actual recording status visible. |
| Verified classified room under the reviewed compatible contract | Recording is prohibited. The pinned Talk25 controller skips recording-consent validation **even when instance-wide consent is required**; do not show a misleading permission-to-record prompt or submit fabricated consent. This exception never authorizes recording and is not inferred solely from a name/version. |
| Missing, conflicting or stale policy evidence | Resolve it before join; do not guess either required consent or the classified exception. |

The [pinned consent implementation](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/CallController.php#L305-L320) establishes the exception, distinct from the [classified restrictions](conversations.md#conv-004). If an actual endpoint requires consent contrary to the expected exception, stop and reconcile/report the discrepancy; do not automatically assent or bypass refusal. Other required OS/security gates remain applicable. For ordinary required consent, declining keeps the user out. A direct call uses a specific consent dialog in its accept flow, never device-prejoin. Joining or disabling a camera is not consent.

Acceptance: publish no media before applicable consent/security gates pass. Test ordinary required/declined consent and verified classified plus global-required consent separately; the latter has no recording-consent prompt and no recording control. Show starting/active/stopping/failed or unknown recording status from current evidence; missing data is not “not recording”. Apply [Q.SEC-10](../quality/security.md#required-bootstrap-policy-decision) to mid-call changes, including those without a changed hash: invalidate on observation, enforce within the approved bound and leave locally when required consent cannot be established at expiry. Consent belongs to the account/room/call generation and policy; a refresh, reconnect or late callback cannot transfer it to a new call.

## CALL-010

**Control server recordings (P3).** Starting/stopping requires `recording-v1`, configured recording support/backend, actual role/permission, and room policy. Explain that this is a server recording. Render resulting recordings and permissions as shared content, including processing, failed upload, and available file states. Audio-only/video options depend on the backend. The recording backend's chunked-upload API is not a desktop-client feature.

Acceptance: a request timeout does not claim recording stopped; reconcile status. A failed backend is visible to all affected native participants when status is available. Classified rooms always prohibit recording under the verified contract, independently of consent defaults or E2EE. E2EE/recording combinations separately follow tested policy. The client does not substitute an undisclosed local recording.

## CALL-011

**Enforce real call E2EE conditionally (P2 safety; full support requires proof).** `call-end-to-end-encryption` and its configuration advertise signaling/policy, not proof of native media encryption. The native engine must implement the compatible encryption/key lifecycle, verify the negotiated state, and surface its actual protection. Transport encryption is not labeled E2EE, and call E2EE says nothing about chat/file storage.

Acceptance: if encryption is required and unsupported, negotiation fails, keys cannot be established, or participants/backends are incompatible, block/leave the affected call with a clear reason. Never downgrade silently. Enforce a newly observed requirement within [Q.SEC-10's approved bound](../quality/security.md#required-bootstrap-policy-decision); full ongoing refresh must include non-hash E2EE changes. Stale/unknown required encryption cannot sustain a call past grant expiry. Fresh policy does not substitute for verified negotiated protection. Recording, SIP, federation, and transcription combinations require their own tested policy matrix; a successful ordinary call does not validate them.

## CALL-012

**Hands, call reactions, and layout (P2 basic; P3 full controls).** Show speaking, hand-raised order/state, mute/video state, focus/gallery, and supported ephemeral call reactions. Use the server's supported reaction list; no option when absent/empty. Respect enforced grid limits and expose accessible participant controls without relying on color or hover.

Acceptance: lost/reordered signaling converges with current participant state; disconnect clears stale speaking/reaction indicators. Grid enforcement follows [Q.SEC-10](../quality/security.md#required-bootstrap-policy-decision), including non-hash changes and expiry: restrict the affected video subscriptions/rendering within the approved bound instead of treating a working media connection as fresh policy. Keyboard and assistive technology can raise/lower hand, focus a participant/share, and leave. Layout changes never silently activate media.

## CALL-013

**Transcription, captions, and translation (P4; status/privacy recognition P2).** Offer live transcription/translation only when advertised, configured, permitted, and supported by the native signaling/media path. Distinguish live captions from a saved transcript and from a recording. Show selected source/target language, processing state, errors, and any required consent/provider disclosure. Do not capture locally or upload to a separate provider as an implicit fallback.

Acceptance: disabled provider, unsupported language, delayed or partial captions, provider failure, and E2EE/classified conflicts produce explicit outcomes. Generated captions are labeled and accessible, can be hidden without implying the server stopped processing, and do not become authoritative chat messages unless a supported deliberate action shares them.

## CALL-014

**Reconnect and end predictably (P2).** On network/control loss, show degraded/reconnecting with bounded retry/cancel even if media transport still works. Policy/control failure cannot renew grants. Under [Q.SEC-10](../quality/security.md#required-bootstrap-policy-decision), stop affected capture/publication by its original expiry plus the approved enforcement bound if complete renewal fails; a source invalidated by disconnect takes effect on observation. Leave locally when membership, required consent or encryption cannot be established; narrow grid restrictions affect subscriptions/rendering. Local stop never waits for a leave acknowledgement. Sleep/wake checks elapsed expiry before resuming.

Confirmed authorization loss of the call's account executes [AUTH-007](accounts.md#auth-007) regardless of live media: invalidate the runtime generation, disable publication, stop/release local capture within the bootstrap deadline, discard queued unsent media and reject late policy/join/media/reconnect callbacks. Best-effort remote leave is independent; another account's session remains intact.

Recovery requires fresh call-existence, membership, permission, consent, encryption and device checks plus valid continuity/user intent. Preserve mute/camera-off state; fresh policy alone cannot unmute, enable a camera or restart a stopped share. After account reauthorization or lost continuity, require deliberate rejoin. An accepted direct call's continuity recovery never introduces conference prejoin; a new conference/webinar/presentation join uses CALL-004, while deliberate Huddle and ordinary group audio rejoin follows CALL-005's direct entry with the same required gates. Show advertised duration policy without promising an exact server end time.

Acceptance: a call ended during outage cannot be resurrected. Observed membership removal invalidates local media authority and stops capture within the approved enforcement/release bounds. [V.AUTH.LOSS](../quality/verification.md#vauthloss--confirmed-account-authorization-loss) and [V.POLICY.FRESH](../quality/verification.md#vpolicyfresh--ongoing-policy-freshness) cover active media with confirmed account loss, surviving RTP with failed control refresh, dropped leave responses, sleep/late callbacks and intent-preserving recovery. Record actual stop times against the approved numeric bounds. End-for-everyone remains a separate authorized action with confirmation; ordinary leave cannot end others' call.
