# Independent product specification review — round 1

**Verdict: changes required before accepting `G.SPEC`.** The reviewed package is a substantial, truthful planning baseline, but three product findings remain open. Resolve PR1-01–PR1-03 and integrate/recheck the outstanding product portions of QR1-01–QR1-03 before handing an accepted specification to the architect. The existing design findings also retain their original review status. This is a document gate; implementing the application or running native/server tests is not a condition for correcting these findings.

Reviewed independently on **2026-10-18**. Scope: all ten `docs/product/*.md` and four `docs/compatibility/*.md` files, including both indexes, every inventory row, every requirement and all 28 compatibility scenarios. The hashes below identify the exact draft. Cross-review/quality material was consulted separately as described below; this report does not approve concurrent quality or design remediation.

Only this new report was written. No authored specification, code or asset was modified; no additional agent, Git operation, VM, service, build, server deployment or publication was used. Findings describe specification counterexamples, not observed failures of a native application.

## Findings

All three findings are **open**. Proposed owner: product/compatibility author, coordinating policy and security wording with the quality author. Severity concerns the specification and its effect on later implementation.

### PR1-01 — Medium: restrictive presets lack explicit invariants and precedence

Locations: [CONV-003/004](../product/conversations.md#conv-003), [CONV-010](../product/conversations.md#conv-010), [CALL-009](../product/calls.md#call-009), F-027–032, and [CT-005](../compatibility/test-matrix.md#ct-005).

The requirements correctly recognize these room attributes, but do not give the architect a deterministic policy contract. “May hide” a channel's participant list and “may prevent” an ordinary announcement member from leaving weaken rules that the reviewed implementation enforces. Preserved-room acceptance covers deletion/history but omits its guest-access and discoverability locks. Classified handling repeatedly delegates to “policy” without enumerating its known rules or separating server requirements from additional native-client restrictions.

The unconditional default → selected → user → forced merge also needs a final invariant step. **Counterexample:** select classified while the returned forced preset enables SIP or discovery. CONV-003's resulting-settings review can describe these as forced-on, whereas the pinned room service sets classified rooms to private/unlisted/SIP-disabled. A classified room also bypasses recording-consent validation in the call controller, even when the instance requires consent elsewhere; CALL-009 gives no precedence case for this. This matters to a truthful settings review and to distinguishing actual required consent from a misleading recording prompt.

The following distinctions need explicit acceptance expectations:

| Area | Baseline rule to encode |
| --- | --- |
| Channel/announcement | Channel calls are disabled for every role; only moderators receive its member list. Announcement members without moderator authority cannot leave; participants receive notifications for all messages by default. |
| Preserved | Guest/public-link and listability changes are forbidden while preserved, as are API deletion/history clearing. Owner-controlled preservation does not defeat classified automatic retention. |
| Classified | Private/unlisted, no SIP or recording, forced sensitive treatment, restricted invitations/external exposure and no unrelated object binding. Specify how these constraints interact with forced settings, consent, retention and an explicit keep/unbind action. |
| Editable preset defaults | Distinguish these from invariants: reviewed webinar/presentation permissions allow joining/chat/reactions without ordinary-member start/publish permissions; voice-room listability and message expiry are defaults. Continue using returned parameters and compatible runtime gates. |

Evidence: the [preset merge contract](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/PresetController.php#L30-L41) describes parameter merging; [RoomService creation](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Service/RoomService.php#L169-L209) enforces classified invariants afterward. [RoomController](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/RoomController.php) enforces member-list, leaving, preserved-settings and sensitive-state restrictions; [CallController consent validation](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/CallController.php#L305-L320) contains the classified exception. [Pinned constants](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/constants.md#conversation-attributes) and [preset fixtures](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/tests/integration/features/conversation-4/presets.feature) corroborate the distinctions. These are source checks, not executed tests.

**Correction/recheck:** add a concise normative rule/precedence table under the existing IDs and matching CT-005/013/018/027 cases. Include classified plus conflicting forced settings; channel member versus moderator; announcement leave; preserved guest/discovery mutation; and classified plus instance-wide recording consent. Reconcile the actual returned room before presenting success. State which cache, preview and export prohibitions are stricter client policy, since those cannot be derived from a generic server permission bit. No architecture signatures or live implementation are needed for this correction.

### PR1-02 — Medium: re-ringing is assigned a moderator-only gate that conflicts with implementation

Locations: [F-133](../compatibility/feature-inventory.md#moderation-and-administration), [MOD-002](../product/moderation-and-integrations.md#mod-002), [CT-016](../compatibility/test-matrix.md#ct-016).

F-133 requires a moderator. The pinned call API prose does say this, but **both** reviewed controllers use `RequirePermission::START_CALL` plus participant/call checks. Middleware tests the effective start-call bit; the target-notification service adds target, session and status checks without adding a moderator requirement.

**Counterexample:** an ordinary member with effective start-call permission, already in an active eligible call, wants to re-ring an eligible absent colleague. The ledger hides this supported action solely because the member is not a moderator. That conflicts with DOM-004/COMP-001's permission-based model and narrows intended parity without identifying a client product restriction.

Evidence: contradictory [Talk25 API prose](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/call.md#send-call-notification); actual [Talk24 controller](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/lib/Controller/CallController.php#L372-L405), [Talk25 controller](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/CallController.php#L381-L414), [permission middleware](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Middleware/InjectionMiddleware.php#L368-L399), and [notification service](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Service/ParticipantService.php#L1592-L1625). This is a concrete documentation-versus-code discrepancy, not evidence of a live-server run.

**Correction/recheck:** document the discrepancy and derive the planned gate from effective permission, active participation and target eligibility. If moderator-only is an intentional client policy, label it as such and record the parity limitation. Add explicit allowed ordinary-member, denied start-permission, caller-not-in-call, target-already-in-call and unavailable/DND-target outcomes; confirm actual response semantics during later baseline integration. A source-link availability check alone cannot resolve this finding.

### PR1-03 — Medium: poll closure is counted but has no actionable acceptance contract

Locations: [F-087](../compatibility/feature-inventory.md#files-and-structured-content), [CONTENT-006](../product/files-and-content.md#content-006), [CT-010](../compatibility/test-matrix.md#ct-010).

F-087 includes poll closure. CONTENT-006 covers creation, voting, reading open/closed states and publishing drafts; its acceptance and CT-010 exercise a vote racing with closure. Neither requires the native user to close a poll or defines that action's authorization and uncertain-completion behavior.

**Counterexample:** a client can pass the listed scenarios by displaying a poll another client closed, yet provide no close action to its author. A hidden-results poll can then be created and voted on here but cannot be completed here. That is a missing promised action, not merely an implementation detail.

The [pinned poll contract](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/poll.md#close-a-poll) permits author/moderator closure; its DELETE endpoint closes a poll rather than deleting the chat object.

**Correction/recheck:** add explicit close behavior and accessible control, author/moderator versus ordinary-voter cases, already-closed/concurrent-close handling, timeout reconciliation and the resulting voting/results state. Distinguish closing from deleting the share/message. Link that criterion from F-087 and CT-010. No new feature ID or executed test is necessary.

## Existing findings carried forward

No finding from the [quality review](quality-spec-review.md) or [design review](design-spec-review.md) is renamed, duplicated under a PR1 ID, or declared closed here. Their original snapshots differ from some current files; source changes alone are not independent remediation approval.

| Existing ID | Product integration or remaining review needed |
| --- | --- |
| **QR1-01 — High** | The consulted revised Q.SEC-03/04 adds generation staging, restoration suppression, uncertain-write cleanup and truthful residual-storage messaging. AUTH-004 still unconditionally describes memory-only credentials/relogin, AUTH-003 permits restoration from an accessible confirmed entry without explicitly requiring committed eligibility, and AUTH-008 says only explicit removal can later delete an existing credential. Integrate the distinction between an abandoned attempted generation and an intentionally retained older item, including late completion/crash/cancel. An uncertain new generation must be cleanable without turning fallback into deletion of an older credential. |
| **QR1-02 — High** | Revised Q.SEC-05/09 adds account-scoped shutdown, generation invalidation and capture/publication teardown. AUTH-007 still only explicitly stops mutations; CALL-014 names membership removal rather than confirmed account rejection. Bind both product requirements and CT-002/015 to the new contract, including local teardown without waiting for a successful network leave, late callbacks and deliberate post-reauthentication recovery. |
| **QR1-03 — Medium** | Revised Q.SEC-10 requires bounded policy freshness, including non-hash changes and healthy media with lost control connectivity. COMP-003/CALL-006/009/014 and CT-004/015/018 must explicitly inherit it; “immediately” must be relative to observed changes and declared freshness/enforcement bounds. Numeric source/expiry/timing decisions belong to `G.BOOT`, not this authoring review. |
| **QR1-04 / QR1-05 — Medium** | Lifetime/peak accounting and frozen workloads remain the quality review's findings. Their remediation and measurement contracts were not independently approved here. No native measurement is required at this product-document gate. |
| **DR-001 — High** | Product CALL-004 already requires voice-room prejoin; preserve that decision while the design owner corrects the shortcuts. This report does not re-test the modified mockups. |
| **DR-002 / DR-003 — Medium** | Product identity/navigation requirements support preserving the room through sharing and prejoin cancellation. Carry the existing concrete design regressions into re-review rather than treating product prose as proof that the mockups are fixed. |
| **DR-004 — Low** | Gallery count/revision cleanup remains a design-owner correction and recheck. |

Cross-reference snapshots are recorded below. The quality edits show substantive remediation work; they do not justify closing the original findings before the product counterparts and final candidate are independently checked.

## Traceability and readiness assessment

The **140 entries are an inventory, not proof of parity**. Independent structural checks found 140 unique F IDs, all **Planned**, and 109 unique requirement/journey/state IDs plus 28 unique CT IDs. Every row links to an existing specification anchor. The 204 local link occurrences and 56 distinct external URLs resolve; external checks establish retrieval, not correctness of every assertion.

The inventory is meaningfully more than a list of capability names: most links reach observable positive/error behavior, and the shared CT permutations cover absence, denial, backend failure, late responses and revocation. PR1-01–PR1-03 identify where that trace is still semantically incomplete or incorrect. A syntactic expansion of CT feature references names 139 inventory entries; **F-182** is not explicitly mapped. Future-OS qualification is already stated in DESK-011 and CT-003, so this is a later trace-maintenance item rather than a reason to demand another platform now.

The 102 named flags in the Talk24 emitter and 106 in Talk25 are all mentioned in the reviewed package. That establishes flag inventory coverage only. Configuration values, invariant precedence, endpoint permissions and complete workflows still need their own acceptance logic.

| Area | Document assessment |
| --- | --- |
| Status and sequence | Clear distinction between target, mockups, source research and runtime evidence. P0/`G.SPEC` precedes Deep Modules bootstrap; `G.BOOT` precedes full implementation. No false native/integration/performance pass found. |
| Identity and room model | Correct unified conversation/chat/files/call domain; account/guest namespace plus room token, typed actors, separate membership/session/media, preserved unknown values and explicit local call generations. The architect must retain local proxy identity and remote host identity, not substitute a bare remote token. |
| Calls and user decision | CALL-001–004/JNY-004/005/CT-011–013 preserve direct colleague acceptance without device prejoin, voluntary conference/voice-room prejoin, zero capture on ringing, explicit media intent and OS/consent/security gates. Cancellation, duplicate invitations and join-can-start-call atomicity are treated as real proof obligations. |
| Offline and lifecycle | Meaningful draft scope, session-only persistence disclosure, delivery-unknown reconciliation, no automatic offline send, separate upload/share completion, full removal reconciliation, sleep/wake/crash/quit behavior and stale-callback rejection. QR1-01–03 still need cross-document integration. |
| Guests/discovery/federation | Guest access, listing and invitation source are separate. Local/host capability scope, proxy support, account isolation, federation direction and remote failure are represented. No blanket “both hosts support everything” or cross-major pairing guarantee was found. |
| Native/API/credential boundary | Native GPUI/no WebView is explicit. Browser login/external providers are identified handoffs; recorder/bot/SIP backend responsibilities are separated. Consulted Q.SEC-01–03 keeps materialized credentials behind authentication/transport/vault adapters and gives GPUI opaque generation-scoped handles. Concrete interfaces remain bootstrap work. |
| Media, accessibility and platform scope | E2EE, recorder, transcription, SIP, effects and optional providers require genuine compatible native/backend paths or a truthful block. VoiceOver, keyboard/focus, reduced motion, text scaling, localization and OS lifecycle have observable targets. None is presented as implemented. |

## Primary-source checks and limits

Read-only HTTPS checks were performed on 2026-10-18. Where the web reader could not retrieve GitHub API JSON, direct HTTPS retrieval of the same official API was used. No upstream code was executed or added to this repository.

- **Release identity:** independently confirmed all four published release records and resolved commits in [baselines and sources](../compatibility/baselines-and-sources.md#published-versions-available-at-review): NC34.0.5 `77e3cab12d2215bf94d6de67f23b98ed7bad236e`, Talk24.0.6 `0d736588dcfb42becf0e5688e0847e35d5b5efa2`, NC35.0.2 `ba900af933e61ffa005315b09d6454bfbbcbcc81`, Talk25.0.6 `eed360d176edcb6888dad60965cb694e6bf62ae3`. Publication timestamps and non-draft/non-prerelease flags match. These are released candidates, not tested client configurations. Evidence: [NC34 release](https://github.com/nextcloud/server/releases/tag/v34.0.5), [Talk24 release](https://github.com/nextcloud/spreed/releases/tag/v24.0.6), [NC35 release](https://github.com/nextcloud/server/releases/tag/v35.0.2), [Talk25 release](https://github.com/nextcloud/spreed/releases/tag/v25.0.6).
- **Research versus release:** confirmed pins `17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79` and `f35255ab86f31e8ee894641f6970a09ce4b90927`, their timestamps and manifests. All **31** existing research Markdown/PHP/XML copies matched bytes freshly retrieved at those pins. The [Talk24 comparison](https://github.com/nextcloud/spreed/compare/0d736588dcfb42becf0e5688e0847e35d5b5efa2...17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79) is 13 commits ahead; the [Talk25 comparison](https://github.com/nextcloud/spreed/compare/eed360d176edcb6888dad60965cb694e6bf62ae3...f35255ab86f31e8ee894641f6970a09ce4b90927) is 17 ahead. The documented room-share-provider versus sidebar/workflow differences are accurate; reviewed capability/preset files are not in the changed-file lists. A branch pin is still not a release artifact.
- **Baseline differences/backports:** verified the preset factory's administrator-only announcement selection and conditional voice-room availability, separate reaction permission 256, added room attributes/owner operations and organization preferences. Talk24's [24.0.1 capability section](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/capabilities.md#2401) supports the recording-chunked-upload, external-call-service and bot-features backport caveat. The ledger correctly avoids making these Talk25-only.
- **Federation and unknowns:** checked the [capability emitter](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Capabilities.php), [room capability endpoint and hashes](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/conversation.md#get-conversation-capabilities), and [global proxy errors](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/global.md). Local-scope lists, host-scoped evidence, 406 unsupported proxy versus 422 remote failure, and conservative unknown-policy handling are sensible specification inputs. Permission/code discrepancies still require feature-specific reconciliation, as PR1-02 demonstrates.
- **Preset/permission semantics:** checked actual factories, preset parameter definitions, room creation/update guards, participant effective permissions, call/consent middleware and the poll contract, beyond feature presence. PR1-01/02 are the resulting discrepancies.
- **Public-file hygiene:** reading and a targeted scan of the 14 reviewed files found no apparent credentials, private-key/token literals or private absolute filesystem paths. This is a limited text check, not a comprehensive security or provenance audit.

The capability snapshot hashes independently match the author evidence: Talk24 `0161726910d4235714c31eb13269b986ee761551bd51c4ab825d28fa80238b95`; Talk25 `2ab5fb2a4339048fe6b570301e6265724be24734a48bb5665d3608cc7ef371cc`.

No native feature, lower supported patch, guest/media topology, Keychain operation, performance number or live federation scenario was verified by this review. The moving Login Flow/user manuals are already identified as moving sources; version-specific endpoint fixtures and runtime checks remain necessary.

## Later gates retained

1. **Specification acceptance:** correct the three new findings, reconcile the named product/quality counterparts and independently recheck design/quality remediation. Bind the accepted draft to the final committed candidate when one exists. Hash-based draft review is not final commit-bound approval.
2. **Deep Modules bootstrap:** define ownership and narrow interfaces for protocol adaptation, account/host identity, action policy, secret-store generations, protected content, native rendering and media. Set cancellation/error contracts, policy refresh/enforcement deadlines, local shutdown ownership, resource limits and test seams. Review the architecture before full implementation; this report supplies obligations, not architecture approval.
3. **Product increments:** implement and verify only the claimed scope on exact NC34/Talk24 and NC35/Talk25 profiles. Preserve negative/unknown/backport/federation cases. Resolve call-accept atomicity, actual native media/E2EE, OS permissions/Keychain, accessibility and lifecycle on suitable environments. Disposable servers are authorized later; they were not needed here.
4. **Release:** declare/test minimum supported patches and OS versions, record exact artifacts and optional backend profiles, qualify performance measurement and frozen workloads, verify signing/distribution/update/disclosure arrangements and update per-feature evidence. Browser handoffs, fixtures, compiling stubs and screenshots cannot establish native parity.

There was no environmental blocker to this review. The remaining blockers are document corrections and independent acceptance, not missing implementation at the authoring gate.

## Reviewed file hashes

Repository-relative paths. The 14 primary-file hashes were captured at **2026-10-18T15:15:18.639984+00:00**, matched the initial reads, and were confirmed again at **2026-10-18T15:23:46.679361+00:00**. All remained unchanged. No Git operation was performed.

| Primary input | SHA-256 |
| --- | --- |
| `docs/compatibility/README.md` | `9045a594228efd6377285053cded8b469110a5d4444903379d5a2c36854178d6` |
| `docs/compatibility/baselines-and-sources.md` | `45c7aaffe254c451b915ce2296f8e8eee531c071bf97d27e813cf9923f2be762` |
| `docs/compatibility/feature-inventory.md` | `5934e711598c3618d5742fa726f01a71bdd936043a5bd0d7b6c11c58841d202f` |
| `docs/compatibility/test-matrix.md` | `dd543c2846c49ab81a02e8d260d504ee674a2b009d4d20d8cd1624c0caf33f69` |
| `docs/product/README.md` | `8d77e6bd03630fc82fb88b9f605c6b401ab57b021de95f0ae94ef9fe5657b832` |
| `docs/product/accounts.md` | `0b2d79a18966e1733703f9137143b5a36748d6fdbd78ca4fc30b50e9b5a5c0f6` |
| `docs/product/calls.md` | `786d665a48ae8405506f386fa1071f21baa72b25f587c4972a602501a7598225` |
| `docs/product/conversations.md` | `265a7e0530f6423fece252362e39c5f94d6792b16e9e3e5483051c4de77a1ea1` |
| `docs/product/desktop-and-accessibility.md` | `87dc4d6b2374dcad10900c95dd2b298a9daaec2e663abf00b7afdf5bb24f140f` |
| `docs/product/files-and-content.md` | `89d175c3ca05a2f2435025c2bfbee21285d660a7cada696c8ad9f5d98bf90589` |
| `docs/product/journeys-and-states.md` | `b670621a1c5e6f3ba51d10524b93953c811508de0e7f8d67ddef44090505a3bc` |
| `docs/product/messaging.md` | `eee59ace54a6105f5729dfbc3aff6b72ff7133af26b061ce4098cd5f76182a87` |
| `docs/product/moderation-and-integrations.md` | `48b6f98ef433e67705caf8fdda9a758566d7bd3f5fa59ce3ccbcbbe7cd25f9cf` |
| `docs/product/terminology.md` | `bbc98a617cff5e317d9bd14d0423f8a4ce580ebd3b8f095e14d4ec4859e0e99f` |

### Cross-reference snapshot

The following are consulted evidence, not another independent review of all quality/design files. Both prior review reports and Q.SEC were read; the quality index and Q.VER gate/traceability sections supplied process context. The snapshot time is **2026-10-18T15:15:18.639984+00:00**.

| Cross-reference input | SHA-256 at snapshot |
| --- | --- |
| `docs/reviews/quality-spec-review.md` | `26c8b5e69c13abcaf595afef9e16a341301b286c943a5606279e239f3cee88f7` |
| `docs/reviews/design-spec-review.md` | `94e47d6a743742f60dde7157023db2307f13b0b08144be80b4e9ace6360da57e` |
| `docs/quality/security.md` | `d73ae798071822059a880d644ae9170fdc39735d52bb7829eba806c33e8a5a35` |
| `docs/quality/index.md` | `cda4671cef04e7a912212acdf618427b7986d2499d26ead3ae61fd3ef0b51876` |
| `docs/quality/verification.md` | `e4c60ce8e61896003a340e69e073eb6b22acdab3dbd2022a94547d29d5b8561a` |

A concurrent Q.SEC update was observed at **2026-10-18T15:20:40.906568+00:00**, with SHA-256 **`4b7506eb4a3ac318752d68548db5d817bd9811ace86de7fbc87250c214ec1691`**. Its intervening changes were inspected: they clarify committed restoration disposition and conservative policy-source age/propagation allowances. They do not change this product verdict or close QR1-01–QR1-03. The initial and later hashes identify which quality wording was consulted. Concurrent performance/design remediation and any later edits require their own review; this report does not silently approve them.

