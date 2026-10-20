# Compatibility and product acceptance matrix

Status: **Planned / not run** for every runtime scenario below. Source verification is recorded separately in [baselines and sources](baselines-and-sources.md). No native client, disposable Talk server, or VM was started for specification authoring. Follow the [quality verification process](../quality/verification.md), [security contract](../quality/security.md), and [test-server guidance](../quality/test-servers.md) when implementation reaches these gates.

Disposable temporary NextCloud/Talk servers are authorized for later functional checks. They must use synthetic accounts/content, reproducible artifacts, minimal exposure, and teardown. VM builds/services use agent-dev-box; actual macOS Keychain, VoiceOver, capture permissions, and devices require an appropriate macOS environment. A Linux VM or mock adapter cannot qualify those platform behaviors.

## Required environments

| Profile | Target | Purpose | Current evidence |
| --- | --- | --- | --- |
| R34 | NextCloud 34.0.5 + released Talk 24.0.6 | Initial published-release compatibility candidate | Release existence verified; runtime not run |
| R35 | NextCloud 35.0.2 + released Talk 25.0.6 | Initial published-release compatibility candidate | Release existence verified; runtime not run |
| L34 / L35 | Earliest maintained patch pair the client release will claim in each family | Establish the lower bound; select/document exact versions before claiming all `.x` patches | Selection pending; runtime not run |
| U34 / U35 | Next maintained patch in each family when released | Upgrade/regression and compatibility-hash refresh | Future run; no assumed compatibility |
| PIN34 / PIN35 | Exact research commits from sources, installed with matching server family | Distinguish branch-source behavior from published releases | Source reviewed only; runtime not run |
| BP | R34/R35 plus controlled capability/backport fixtures | Advertised backport enabled; missing capability on a newer patch; no major-only switches | Fixture/test implementation pending |
| UNK | Unknown feature strings, room types/attributes, roles, object kinds, malformed or absent fields | Safe forward compatibility and restricted-policy handling | Fixture/test implementation pending |
| FED | R34↔R34, R35↔R35, R34↔R35 with both hosting directions | Local/host scopes, proxy support, identities and remote failure | Runtime not run |
| NATIVE | Supported macOS hardware/OS builds, actual GPUI and platform adapters | Keychain, devices, permissions, notifications, sleep, VoiceOver | OS/version minimums and evidence pending |
| STORE-LINUX | Named Linux OS/filesystem and Secret Service implementation; separately no session D-Bus and D-Bus without Secret Service | Credential-provider contract only: native lock/denial, durable private-file fallback, `0700`/`0600`, restart and backend choice | Specified / not run; does not qualify a Linux UI/call client |
| STORE-WINDOWS | Named Windows OS/filesystem, user SID, Credential Manager and actual file DACL APIs | Credential-provider contract only: local persistence, denied access, owner/DACL/inheritance/reparse checks and file restart | Specified / not run; does not qualify a Windows UI/call client |
| CONDITIONAL | Named HPB, TURN, recorder, transcription/translation provider, SIP bridge, bots/bridges | Optional deployment behavior; pin each component separately | Configuration and runtime evidence pending |

R34/R35 are research-date candidates, not frozen forever or already supported client versions. A newer maintained patch replaces the current candidate only with a recorded change and relevant revalidation. Do not install incompatible NextCloud/Talk major combinations and then label the resulting failure client parity; use those only as explicit negative discovery tests.

## Common permutations for every delivered feature

Run the positive path on R34 and R35 wherever the feature's contract exists; for a family-specific feature, run the supported path there and the absent-feature behavior in the other family. Also cover capability absent, config disabled, backend unhealthy, permitted/denied role, permission revoked in flight, offline/timeout, duplicate or delayed response, and unknown optional data where relevant. Record explicit not-applicable reasons; no silent skips.

Conversation fixtures include direct, group, public listed/unlisted, password protected, note-to-self, former direct/changelog, voice/webinar/presentation, read-only, sensitive, file/event/phone, breakout child, external call, and unknown type/attribute. R35 adds classified, channel, announcement, and preserved. Actor fixtures include owner, moderator, ordinary user, guest, guest moderator, federated user, bot, and bridged user. Users belonging to allowed/disallowed start-call/SIP groups are separate fixtures.

Native call tests cover direct and multi-party calls, internal and external signaling where supported, direct media and forced TURN, microphone/camera/screen permissions in granted/first-prompt/denied/managed states, multiple devices, muted/camera-off reconnect, and adverse network conditions. Recording/E2EE/transcription/SIP combinations are not automatically valid: prove the allowed combination or verify the correct block.

## Scenarios

### CT-001

**Baseline identity and API discovery.** On R34/R35, record exact NextCloud/Talk artifact versions and digests, manifests, capabilities/config-local scope, endpoint generations, installation base path, and client revision. Exercise missing Talk, unsupported versions, incompatible pairing metadata, malformed OCS, and missing required API support. Expected: COMP-001/005/006 hold; no endpoint/version guess grants an action. Covers F-001, F-190/191.

### CT-002

**Browser authorization and account isolation.** Complete ordinary, 2FA, and configured SSO login; cancel, expire, deny, lose the one-time poll response, and inject a late success. Use two users on one server, two installations under one host, two servers with identical room tokens, and an isolated guest session. Confirm account rejection during pending sends/transfers and an active call; deliver old-generation login/request/policy/media/reconnect successes and drop the remote leave response. Expected: AUTH-001/002/006/007 and [V.AUTH.LOSS-01–03](../quality/verification.md#vauthloss--confirmed-account-authorization-loss) hold: authenticated admission closes, local publication/capture stops within the approved bound independently of network/vault success, and stale callbacks cannot revive it. Other accounts and permitted drafts survive. Distinguish action denial, room/session expiry and transient errors; fail durable restore suppression and require its warning. New authorization alone neither retries a send nor rejoins a call; another user inherits no work. Covers F-002/006–008/011 and JNY-001/007/008.

### CT-003

**Native stores, durable unencrypted file fallback and cleanup.** Apply [AMEND-AUTH-STORAGE-01](../product/accounts.md#spec-amendment--amend-auth-storage-01) and every [V.AUTH.STORE-01–16](../quality/verification.md#vauthstore--uncertain-storage-transitions) case. On NATIVE qualify actual Keychain plus private-file behavior. On STORE-LINUX test named Secret Service success/lock/denial, no session bus and a bus without a service. On STORE-WINDOWS test user-scoped Credential Manager with local persistence and actual owner-restricted DACL enforcement/failure. All scenarios remain **specified / not run** here.

Select file mode, validate a synthetic app credential against R34 and R35, exit and restart without the OS store: the committed file binding restores without forcing a new login just because a service is absent. Recover/unlock the native store and restart again: no implicit migration/save or abandoned-generation revival. Test explicit file↔native migration, later session save and cancel-and-retain; never infer equivalence from a common account name or generation number. An existing valid memory session may continue; an unreadable P requires fresh browser authorization or available validated memory, never a primary password. No file encryption/master-password/KDF gate is required.

Exercise wrong directory/file owner, POSIX `0700`/`0600` violations/extra ACLs, unsafe parent/XDG paths, symlinks/hard links/traversal, Windows null/broad/inherited DACL/readback failure and reparse points, shared/synced/temp locations, concurrent processes, collision, short/disk-full/flush/install writes, corrupted/oversized/staging/orphan files and failed metadata. Apply all prior boundaries: commit without acknowledgement, activation failure, cancellation/fallback plus late writes, crash either side of each provider/ledger boundary, inaccessible retained P and early absence before writer settlement. Distinguish one actual native prompt from an injected locked/denied error; verify no repeated background prompts.

Expected: only committed **account/backend/item** eligibility plus the same confirmed accessible item restores. Session-only explicitly suppresses every generation/backend before replacement activation; failed suppression blocks that replacement with repair/cancel. Exact abandoned-N cleanup cannot touch P or a different backend's matching generation; suppression, late-writer uncertainty and account-loss shutdown survive file fallback. Offline logout/removal and confirmed revocation cover file and native credentials separately from remote deletion success. File mode permits only its validated private credential file/staging objects; logs, environment, clipboard, shared caches and recovery exports contain no secret. Session-only creates no new secret file or durable content cache. No false memory-only/encrypted/saved/deleted claim.

Covers F-003–005/009/010/012 and the future credential portion of F-182. Additional OS clients remain Planned/P4: provider preparation or qualification alone cannot pass native UI/capture/accessibility/notification support. Record OS/provider/filesystem evidence separately from live backend authorization and fault fixtures; macOS, Linux lab or mock results cannot qualify another OS.

### CT-004

**Capabilities, hashes, backports, and unknowns.** Use BP/UNK plus both live families. Advertise a compatible optional feature on the older family; remove it from the newer. Change local/host hashes, forced presets, calls, reaction permission, consent and unknown attribute bits during pending actions. During uninterrupted media, separately change E2EE/grid policy without a hash change, withhold/partially fail refresh, delay old-generation responses and sleep past expiry. Expected: COMP-001–004/006 and STATE-003/005; no version-only gate, unsafe default, erased unknown bit or cross-account invalidation. Apply [V.POLICY.FRESH-01–04](../quality/verification.md#vpolicyfresh--ongoing-policy-freshness): hash matches/RTP/heartbeats renew no unrelated property; record authoritative source, source age, expiry and enforcement against approved numeric bounds. Without complete renewal, suspend/leave the affected path by its bound. Missing source/timing decisions block `G.BOOT`, not justify an invented interval or runtime pass. Covers F-025/027/037/105/113/115/117/120/191/195.

### CT-005

**Conversation creation and policy.** On both families create direct/group/public/note-to-self; re-open a returned existing direct room; invite a directory group independently of room identity. Test returned default/selected/user/forced merging followed by server invariants, no-create policy, password enforcement, timeout reconciliation and listed/unlisted guest combinations. Verify returned webinar/presentation ordinary-member join/chat/reaction permissions independently of start/publish, and voice-room editable discovery/expiry defaults plus deliberate direct audio entry and its required gates.

On R35 (and any advertised compatible backport), exercise each restrictive case:

- Classified plus conflicting public/listable/SIP values, including forced settings: review private/unlisted/no-SIP; reconcile actual returned room and partial invitations. Nonempty unrelated object binding is rejected, never silently stripped to create a different requested object.
- Channel owner/moderator/member: no calls for any role; only moderator authority receives the member list. Changing default posting permission cannot enable calls or disclose the list.
- Announcement: administrator-only creation, inherited channel rules, ordinary-member leave denial, all-message notification default and permitted personal override.
- Preserved: only owner toggles it; deletion/history/public-private/listability changes fail while set. Unpreserve requires confirmation before these actions become available and does not lift classified invariants.
- Classified: reject email/phone/federated invite, bot/bridge enablement, chat summary/private-reply leakage and attendee export; force sensitive treatment. Separate one-hour message default from configured room retention and CT-027's explicit keep transition.

Expected: CONV-002–005/010; no unrestricted intermediate room, blind create retry or success based on submitted values. R34 without those attributes exercises absence/unknown restrictions rather than fabricating support. Covers F-021–032/044/045 and JNY-003.

### CT-006

**Discovery, participants, and personal organization.** Exercise successful/empty/failed search, joining a result that becomes private, user/group/circle/email/federated identities, CSV partial failure, hidden participant list, guest naming, role/owner changes, favorites/archive/tags/importance/sensitive/sort preferences. Test Talk-25-only settings absent on R34 and vendor fixtures. Expected: CONV-001/006–009 and DOM rules; no leaked list or privilege inferred from role. Covers F-020/026/033–043/046/138.

### CT-007

**Message delivery, drafts, and privacy.** Send formatted Unicode/IME text, reply, mention, silent message, edit/delete, react, read/unread; test note-to-self edits and chat-denied/react-allowed participants. Disconnect before upload/request, after request but before response, and during reconciliation; deliver duplicates/out-of-order events. Expected: MSG-001–005/007–010 and DESK-007; one canonical message or truthful unknown delivery, no automatic offline send, no hidden read/typing privacy leak. Covers F-050–055/057–064 and JNY-002/008.

### CT-008

**Threads, pins, reminders, schedules, and context.** Create/rename/subscribe to threads; pin/unpin/hide/expire; set/list/cancel reminders; schedule/edit/cancel messages, including quit and DST boundaries. Delete/expire/revoke the target during navigation or mutation. Test server failure and ambiguous schedule creation. Expected: MSG-006/011–014/016; audience and time are correct, hidden pins do not disappear globally, retries do not duplicate schedules, no retained secret excerpt. Covers F-056/065–068/071 and JNY-009.

### CT-009

**Files and media.** Upload multiple files, reuse existing files, use conversation subfolders, captions/thread metadata, previews/downloads, shared-item browsing, and unsupported formats. Exercise no quota, forbidden attachment, path conflict, renamed/missing source, upload success/share failure, canceled downloads, and access revocation during preview. Expected: CONTENT-001–004/007; separate upload/share states and no forbidden cache revival. Covers F-080–085/089/091.

### CT-010

**Voice messages and polls.** Verify native-to-official-client playback and the reverse with selected codecs; deny/revoke microphone, cancel recording and interrupt upload. Exercise anonymous/named polls, multiple-choice limits and moderator draft/edit/publish. Execute the native accessible **Close** action as author and as moderator; deny an ordinary voter and an actor losing authority/access/lobby eligibility. Close an already-closed poll, close from two clients concurrently, and lose the successful response. Fetch state before deliberate retry: closed converges to final results, open remains retryable only with current authority, unavailable remains unavailable, failed refresh remains uncertain. Race a vote with closure.

Expected: CONTENT-005/006; confirmed closed polls retain their message/share and disable voting/Close. Hidden/anonymous results reveal aggregate counts without voter identities; public results obey actual access. Closing a published poll via DELETE is distinct from deleting its message or a moderator draft. Neither a timeout nor a missing result is zero votes or success. Covers F-086–088; merely displaying a poll closed elsewhere does not satisfy F-087.

### CT-011

**Direct accept versus conference prejoin.** Receive a direct call with camera off and known microphone permission. Accept from the app and OS notification. Repeat with start-without-media and explicit video acceptance. Then deliberately join a conference/webinar/presentation and cancel its prejoin. Separately use the headphone action for a Huddle and ordinary group, verify direct audio entry without generic prejoin, and cancel a required permission/consent gate. Verify real Channel call denial for every role. Expected: CALL-001/004/005; direct accept never displays “Bereit, wenn du es bist.”, conference join does, and ringing/selection/restore captures nothing. Test focus safety while typing Enter in another view. Covers F-100/103/104/106 and JNY-004/005.

### CT-012

**Direct acceptance race orderings.** Use deterministic adapter events plus real R34/R35 signaling checks: remote cancel before accept; between validation and join; during permission prompt; during consent; immediately after join response; duplicate accepts; accepted elsewhere; stale notification after a new call; decline with network failure; existing other call. Expected: CALL-003; cancellation invalidates late work, obsolete joins leave immediately, media never activates after cancellation is observed, and no intentional call restart occurs. Record any server atomicity limitation as a blocker to the claimed behavior. Covers F-100/102 and JNY-004.

### CT-013

**Direct-call OS/consent/security gates.** On NATIVE test microphone/camera first prompt, denied, restricted, absent/busy and revoked. Combine with ordinary-room consent required/declined/changed, lobby/join denial and encryption-required unsupported/negotiation failure. Expected: CALL-002/009/011; receive-only only if allowed and working, no direct-call device-prejoin, no false active-media state or security downgrade. Cancel remotely while System Settings is open; old consent/OS callbacks cannot authorize a later call. Separately join a verified classified conference under global-required consent: retain voluntary prejoin, prohibit recording and skip the inapplicable recording-consent prompt. Conflicting/unknown classification or unexpected server consent refusal blocks for reconciliation; no fabricated assent or bypass. Covers F-031/101/104/115/117.

### CT-014

**Native media and screen sharing.** On both families verify direct and group calls against official clients using supported internal/HPB and TURN topologies. Change devices, disconnect a headset, change output route, revoke capture, cancel a picker, close a shared window, stop sharing, and remove the participant. Expected: CALL-006–008/012; correct flags and actual media, persistent stop controls, no silent capture source migration. System-audio/effects tests run only for a declared supported path and remain unavailable otherwise. Covers F-105/107–114.

### CT-015

**Network, lifecycle, and end-of-call.** Degrade/drop connectivity; sleep/wake past policy expiry; replace room session; close window; quit/crash; remove membership/end call during outage. Separately cut policy/control refresh while RTP remains live. Record original grant expiry and actual suspension/leave/device release against Q.SEC-10's approved bounds; failed refresh/heartbeats/media receipt never extend authority. Restrict grid subscriptions/rendering where appropriate; locally leave when membership/required consent/encryption cannot be established, even without a successful leave response.

Confirm account authorization loss during media and pending sends, deliver late join/policy/media/reconnect callbacks, and fail durable restoration suppression while another account stays active. Expected: AUTH-007/CALL-014/COMP-003/DESK-007/008 plus [V.AUTH.LOSS](../quality/verification.md#vauthloss--confirmed-account-authorization-loss) and [V.POLICY.FRESH](../quality/verification.md#vpolicyfresh--ongoing-policy-freshness): invalidated generations never resume; local shutdown is independent of network/vault success, storage uncertainty is visible, unrelated accounts survive. Reauthorize and require deliberate retry/rejoin with fresh checks. Ordinary continuity recovery preserves mute/camera-off intent and never restarts a stopped share; direct continuity has no conference prejoin, new conference entry does; explicit Huddle/group audio rejoin omits generic prejoin but retains required gates. Test duration policy, ordinary leave versus authorized end-for-everyone and no ended-call resurrection. Covers F-008/100/104/120/121/178 and JNY-008.

### CT-016

**Lobby and participant moderation.** Test owner/moderator/user/guest/guest-moderator policies, timed lobby, supported individual/bulk admission, force mute, remove/ban/unban and publish permission revoked in flight. Re-ring cases on R34/R35 must include an **allowed ordinary member with effective START_CALL already participating in an active call**, denied/revoked start permission (including a familiar role label), caller not in call, ended call, unsupported guest/group/phone target, missing/wrong-room target, already-in-call target and local DND target. Cover federated target/proxy support and unavailable recipient/transport without inferring DND remotely.

Expected: MOD-001/002; no remote unmute or guessed moderator authority. Compare [S21 implementation versus API prose](baselines-and-sources.md#re-ring-authorization-and-response-discrepancy): already-in-call may be a successful no-op, DND is target-level refusal, and request success is not delivery proof. Test uncertain completion without repeated automatic rings or new-call creation. Record actual responses on each profile before parity claims. Covers F-130–133/139 and JNY-006.

### CT-017

**Breakout lifecycle.** Configure each supported assignment mode, add/delete rooms, start/stop, move a participant, broadcast, request help, and disconnect/end parent during transition. Expected: MOD-003; correct parent/child room identities, destination policies, no duplicate media publication or wrong-room draft send. Covers F-134–136.

### CT-018

**Recording lifecycle and policy combinations.** With and without recorder/HPB, test start/active/stop/failure, timeout during stop, eventual shared recording, required/declined consent and each permitted E2EE/SIP/transcription combination. Verify classified plus global-required consent skips only the inapplicable recording-consent gate while still prohibiting recording/SIP and retaining OS/security checks. Unknown/conflicting policy never grants that exception; unexpected required-consent refusal is reconciled, not bypassed.

Change consent/recording/E2EE policy during uninterrupted calls, including absent hash invalidation; interrupt policy refresh with live media and deliver delayed/old-generation consent responses. Expected: CALL-009–011/014 and COMP-003 inherit [Q.SEC-10](../quality/security.md#required-bootstrap-policy-decision) and V.POLICY.FRESH: observed restrictions invalidate grants, expiry/enforcement follows approved numeric bounds, unmet required consent/encryption causes local leave, and consent cannot transfer to a different call. No false stopped status, undisclosed local fallback or unsupported protection claim. Recorder chunking needs server deployment evidence, not a native upload control. Covers F-031/115–117/120/193.

### CT-019

**Call encryption interoperability.** Prove native key negotiation, participant changes/rejoin, failure/downgrade rejection, and actual media protection against each declared compatible official-client/backend combination. Verify labels distinguish transport encryption, call E2EE, sensitive/classified rules, and ordinary chat/file storage. Expected: CALL-011; unsupported requirements block the call. A mocked encrypted flag is insufficient. Covers F-117 and conditional portions of F-151.

### CT-020

**Transcription, translation, AI, and provider failure.** Test supported/absent/disabled providers, language selection, pending/partial/failed tasks, consent and room restrictions, and original-versus-generated content. Confirm hiding captions does not claim server processing stopped. Expected: MSG-015/CALL-013/INT-008; no automatic external content transfer or fabricated provider support. Covers F-069/070/118/119/163.

### CT-021

**Federation both ways.** On every FED pair exercise invite/accept/reject/chat/context/reactions and supported files/calls; diverge local/host configuration, disable trust, remove permissions, change proxy hash, and fail remote transport. Expected: INT-001 and COMP-002/003; 406 unsupported-proxy versus 422 remote failure are distinguished, no credential forwarding or identity collision. Covers F-150–152 and JNY-007.

### CT-022

**Bots, commands, bridges, and external services.** Test approved bot enable/disable/output, rejected command, restricted-room exposure, disabled Matterbridge, unknown provider workflow, and external_call room. Expected: INT-002/006/007/008; no shared-secret requirement for ordinary client bot management, no embedded iframe/JavaScript, unsupported native behavior clearly labeled. Browser handoff is tested separately and never counts as native call success. Covers F-090/153/154/161–163/195.

### CT-023

**SIP and phonebook.** With configured bridge, test PIN/PIN-less dial-in policy, permitted/denied SIP groups, number/region validation, rejected/busy dial-out, timeout/duplicate request, returned phone-room association, and classified/E2EE restrictions. Expected: INT-003/CONV-006; no duplicate dial-out or exposed PIN, no inferred E2EE, phonebook query is deliberate. Covers F-046/155–157/194.

### CT-024

**Calendar, file associations, verification, and exports.** Schedule meetings across timezones/DST, inspect mutual calendar data with limited permission, open file-linked chats, keep/unbind temporary rooms, revoke a public share during verification, and export attendance/import CSV with malicious cell contents and partial failures. Expected: INT-004/005/MOD-004; no accidental call/permission grant or inferred identity verification. Covers F-035/044/137/158–160.

### CT-025

**Notifications, presence, and desktop settings.** Test OS allow/deny/Focus, room notification/call levels, silent send, privacy previews, session active/inactive, deep links, multiple accounts, presence expiry, profile/avatar errors, and local/server settings conflicts. Verify actual running/background/quit delivery limitations. Expected: DESK-002–004 and MSG-009/010; no false delivery or presence promise. Covers F-042/171–174.

### CT-026

**Native accessibility and localization.** Complete JNY-001–010 as applicable using keyboard and VoiceOver; inspect focus, dialog recovery, announcements, reduced motion/contrast, 200% text, small windows, English/German/Spanish UI and bidirectional/IME input, timezone presentation, and compact-call controls if implemented. Deterministic localization cases cover complete keys/resources, placeholder compatibility, locale matching and English fallback, formats/plurals, explicit preference persistence and failed-write rollback/Retry. Native Linux cases cover all three locales at narrow widths and 200% text, localized actions/errors/hints/notifications and safe live switching with account/call/draft/focus retained; content and protocol IDs remain unchanged. Linux CI and native evidence do not qualify macOS VoiceOver or platform behavior. The user tests macOS only after all agent-side implementation, integration, reviews and delivery work. Expected: DESK-001/004/005/006/010; no inaccessible critical action or accidental keyboard acceptance. Covers F-122/170/175/176/180.

### CT-027

**Retention and information boundaries.** Expire/delete/revoke content while it appears in a timeline, reply, pin, notification, file preview, search result, draft, accessibility announcement or export. Clear cache/remove an account with pending requests. Exercise CONV-004's stricter classified client policy: memory-only in-room rendering, no external excerpts/provider handoff/clipboard/forward/export or durable content caches, including late preview results and an already-cached room becoming restricted. Distinguish these client rules from specific server endpoint guarantees.

For classified retention, separately vary message expiry, configured room retention (including disabled sweep), owner preservation and moderator keep. A preserved queued classified room still follows automatic room retention. Confirm keep returns `classified_persist`, leaves classified/sensitive/exposure/message-expiry rules intact and excludes later calls from re-queuing under the verified contract. Ordinary members cannot keep; timeout or deletion racing keep reconciles before success. No exact cleanup deadline or erasure of previously user-saved files is promised. Expected: AUTH-009/CONV-004/010/CONTENT-007/STATE-005/006; no protected-content revival or wrong-account fallback. Use synthetic secret markers. Covers F-010/031/032/043/044/091/177/179.

### CT-028

**Operational errors and honest status.** Inject malformed response, 401/403/404 with endpoint-specific meanings, 406/422 federation errors, 426 upgrade required, 429 limits, 503 maintenance, and uncertain mutation completion. Expected: STATE-001–007/COMP-005/DESK-009/011; bounded retries, retained intent, no empty-as-error or click-as-success, redacted diagnostics. Covers F-179/181/183/190/192.

## Evidence and release decision

For each scenario record profile(s), test class (fixture/native adapter/live integration/manual accessibility), client revision, test implementation reference, result (`not run`, `pass`, `fail`, `blocked`, or justified `not applicable`), redacted artifacts, unresolved findings, and reviewer. Use the quality acceptance report template rather than overwriting this planned matrix with an unqualified checkmark.

P1 requires applicable CT-001–009/025–028 safety paths; P2 additionally requires CT-011–016/018–019 for every call policy the client will encounter, including correct blocking of unsupported combinations. P3/P4 add their feature scenarios and regress the affected earlier cases. A deferred feature's restrictive policy still needs testing when its data can reach an earlier phase. Publication of a reviewed documentation/bootstrap repository does not require pretending these runtime tests ran; shipping or claiming a working feature does require its evidence.
