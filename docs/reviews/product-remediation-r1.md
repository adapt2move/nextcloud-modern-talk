# Product specification remediation — round 1

Author handoff, **2026-10-18**. Scope: `docs/product/**`, `docs/compatibility/**` and this new report. The [independent product review](product-spec-review.md), [original quality review](quality-spec-review.md) and [quality remediation](quality-remediation-r1.md) were read in full. Earlier reports, quality/design documents, license and code were not edited.

**Status: corrections submitted for independent re-review.** PR1-01–PR1-03 and the product portions of QR1-01–QR1-03 are author-corrected, pending verification; this report does not independently close them or accept `G.SPEC`. The separately completed [quality round-2 review](quality-spec-review-r2.md) verifies only the hashed quality-text corrections and explicitly leaves product integration pending. No native client, server interoperability or performance result is claimed.

## Product finding responses

| Finding | Correction and exact requirement IDs | Acceptance trace |
| --- | --- | --- |
| **PR1-01 — restrictive presets and precedence** | [CONV-003/004](../product/conversations.md#conv-003) adds parameter merge → invariant enforcement → current gates → actual returned-room reconciliation. Enumerates channel all-role call denial/member-list restriction, announcement leave/default notification behavior, preserved API locks, classified access/SIP/recording/sensitive/exposure/object rules, and editable webinar/presentation/voice defaults. [CONV-010](../product/conversations.md#conv-010) separates owner preservation from moderator classified keep, retained restrictions and automatic retention. [CALL-009/010](../product/calls.md#call-009) distinguishes the verified classified/global-consent exception from required consent and unconditional classified recording prohibition. Stronger native cache/preview/export policy is explicitly client policy. | F-027–032/115; CT-005/013/018/027. Conflicting forced listable/SIP settings cannot appear as permitted; object conflicts reject; returned contradictions block/reconcile. Channel moderator/member, announcement leave, preserved guest/discovery mutations, classified global consent, expiry and keep races are explicit cases. |
| **PR1-02 — re-ring permission** | [MOD-002](../product/moderation-and-integrations.md#mod-002) and F-133 use effective START_CALL, current active participation, eligible call and target gates. No added moderator-only policy. [Source discrepancy](../compatibility/baselines-and-sources.md#re-ring-authorization-and-response-discrepancy) records prose versus both controllers and the successful already-in-call no-op. | CT-016 adds positive ordinary-member, denied/revoked start permission, caller outside/ended call, invalid/wrong-room actor, already-in-call, DND and unavailable/federated target cases. No success-as-delivery claim or automatic retry ringing. Actual released-profile responses remain to be tested. |
| **PR1-03 — actionable poll closure** | [CONTENT-006](../product/files-and-content.md#content-006) and F-087 require accessible author/moderator Close, current access/lobby gates, pending/uncertain reconciliation, final voting/results state and preservation of the chat/share. Published poll closure, moderator draft deletion and message deletion are distinct despite the DELETE verb. | CT-010 requires the native Close action as author and moderator, denial as ordinary voter/after access loss, already/concurrently closed and lost-response cases, aggregate versus public results, and concurrent voting. Displaying a poll closed elsewhere is insufficient. |

The [source catalog](../compatibility/baselines-and-sources.md#protocol-sources) adds S20–S22 and replaces the moving Login Flow reference in S17 with NC34/NC35 documentation. F-182 now has an explicit CT-003 trace for future platform qualification; it remains **Planned/P4**, and macOS/mock results cannot qualify another OS.

## All six quality-to-product amendment groups

The quality contracts remain authoritative; the product text specifies visible outcomes and links their detailed transitions/scenarios rather than copying the technical contract.

| Group from quality remediation | Product-side correction | Existing QR finding and checks |
| --- | --- | --- |
| **1 — committed restoration eligibility** | Accounts introduction uses NC34/35 Login Flow. AUTH-003 requires committed account/generation eligibility **and** a confirmed accessible item; staging/unresolved intent suppresses restoration. Failed pre-write metadata dispatches no write; failed activation metadata is not stored-login success. | QR1-01; CT-003, V.AUTH.STORE. |
| **2 — truthful session-only fallback** | AUTH-004 requires explicit choice and durable account-wide suppression before replacement activation. Known non-write/cleaned-up storage differs from an uncertain earlier write. Residual cleanup remains visible through late completion/crash/restart; failed suppression offers repair/cancel without an unsupported restart promise. | QR1-01; CT-003 includes all six V.AUTH.STORE cases. |
| **3 — attempted generation versus retained older item** | AUTH-005/008 permit exact abandoned-attempt cleanup after writer settlement without deleting the retained older credential. Only a committed deliberate cancel-and-retain disposition restores the named valid older generation. Authorized replacement/removal has its own scope. Metadata failure and restart uncertainty remain visible. | QR1-01; CT-003 covers old-item lock/retention, late success, early absence and cleanup. |
| **4 — confirmed account rejection** | AUTH-007 invalidates runtime generation and authenticated admission, cancels account work, stops local capture/publication independently of leave/vault success, rejects late callbacks and suppresses all stored-generation reuse. Distinguishes action/session/network errors, preserves other accounts/permitted drafts, discloses failed durable suppression and requires deliberate retry/rejoin after new authorization/fresh checks. | QR1-02; CT-002/015, V.AUTH.LOSS-01–03. |
| **5 — ongoing policy freshness** | CALL-006/009/011/012 and COMP-003 inherit Q.SEC-10 for uninterrupted calls and non-hash permission/consent/E2EE/grid changes. Unknown/stale values grant no new authority; observed restrictions and expiry enforce within the approved bounds. Hash matches, RTP and unrelated traffic cannot renew policy; consent does not transfer between call generations. | QR1-03; CT-004/013/018 and V.POLICY.FRESH. Numeric/source decisions remain mandatory at `G.BOOT`. |
| **6 — local shutdown, live-media control loss and recovery** | CALL-014 binds account-loss shutdown and late-callback rejection, expiry with working RTP, local leave without acknowledgement, sleep/wake and intent-preserving recovery. Fresh state alone cannot unmute, enable a camera or restart a share. Reauthorization/lost continuity needs deliberate rejoin. | QR1-02/03; CT-015/018 plus V.AUTH.LOSS and V.POLICY.FRESH. |

Cross-cutting consistency updates: product README canonical storage decision; JNY-001/003/008 and STATE-005; F-003/004/008/027/028/030–032/087/115/120/133/182/191; CT-002/003/004/005/010/013/015/016/018/027. CALL-001–004, JNY-004/005 and CT-011/012 retain direct acceptance without device-prejoin, voluntary conference/voice-room prejoin and no capture on ringing. Required OS/consent/security gates remain intact.

QR1-04/05 concern quality-owned workload/peak measurement contracts. No product-side measurement claim or correction is made for them; their separate quality round-2 disposition and later bootstrap/native obligations remain in force. Design findings/remediation are outside this author's ownership and are not declared closed.

## Source evidence and decisions

Focused read-only primary-source checks on 2026-10-18 used the existing immutable research pins: Talk24 `17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79` and Talk25 `f35255ab86f31e8ee894641f6970a09ce4b90927`. These branch-source pins remain distinct from the published release candidates.

- **Preset rules:** inspected Talk25 RoomService/RoomController, preset parameters/constants/factory/merge, participant defaults, invitation guards, ChatController, BotController, MatterbridgeController, call consent/attendance/SIP guards, classified call-end listener and object-retention job. The [precedence evidence](../compatibility/baselines-and-sources.md#preset-and-policy-precedence) links those exact sources. Ten existing Talk25 research snapshots (constants, settings, merge controller, factory and six presets) matched freshly retrieved bytes. No upstream files were copied into the repository or executed.
- **Re-ring:** checked both pinned CallControllers and ParticipantServices plus permission middleware. The service does not add moderator authority; already-in-call suppresses notification with possible success, local DND returns a target refusal. Federated status/delivery remains profile-dependent. The native client requires established active participation; source review is not a universal server guarantee for every session/proxy path.
- **Poll Close:** checked the documented contract, authorization service, closed-state response, draft branch and result rendering. PollController and PollService are each byte-identical between the two research pins: SHA-256 `5703cb0e7d5ebee408713cd4e70a84937883adfd707eec2df2dafe896649ce48` and `ae66f882dac82f6d7951325de911241ecca6e7e9ec9cdf6c45382ea6505202a5`, respectively. [S22](../compatibility/baselines-and-sources.md#protocol-sources) links both families.
- **Login/freshness:** retrieved the version-specific NC34/35 Login Flow manuals through direct HTTPS after the web reader returned “Internal Error”; both contain Login Flow v2 and app-password deletion sections. Rechecked pinned settings for non-hash E2EE/grid changes. This supplies documentary evidence, not tested client refresh routes or a completed authentication run.

Product interpretation made explicit: classified native excerpts, external processing, clipboard/forward/export and durable content caches are restricted beyond specific checked server guards, while authorized in-room native rendering/accessibility may use memory. This continues the requested restrictive handling; it does not promise prevention of participant screenshots, server/plugin exposure or deletion of previously exported files. Preset defaults and server invariants are not interchangeable.

No unresolved product choice required guessing in this round. The following remain **gate obligations**, not silently chosen defaults:

1. Independent product re-review and coordination with the separate quality/design conclusions before whole `G.SPEC` acceptance; final committed-candidate confirmation when a commit exists.
2. At `G.BOOT`: concrete credential staging/durability/crash interfaces, account-rejection classifier/shutdown ownership and numeric device-release deadline; authoritative policy source mapping and numeric freshness/refresh/request/propagation/enforcement bounds, recovery and control-traffic costs. No numeric interval was selected here. An unavailable/unbounded required source blocks the affected path/support claim.
3. Later native/integration gates: both released server profiles and supported backports, actual Keychain and native devices/media, direct-accept atomicity, re-ring response semantics, classified/poll behavior, accessibility and performance evidence. No full application implementation is authorized by this handoff.

## Author verification

Read-only standard-library scans and manual contract review; these are documentation checks, not application tests.

- **109 unique requirement/journey/state IDs** and **28 unique CT IDs** are retained. **140 unique F IDs**, all **Planned**; every inventory row links a requirement. Expanding CT feature ranges now references **all 140** with no unknown feature references.
- **254 local link occurrences** across the 14 product/compatibility files and **13** in this report resolve to existing files/heading anchors. **78 distinct external URLs** were counted; only the focused primary sources above were re-fetched, not every external link.
- No matches in the authored scope for the checked private absolute-path, credential-literal/private-key or moving Login Flow URL patterns. This limited scan is not a comprehensive security audit.
- Manual reconciliation covered forced settings versus invariants; ordinary required consent versus the classified exception; preserved versus keep/message retention; ordinary-member re-ring and target no-op/denial; Close versus deletion; retained older credentials versus attempted-generation cleanup; confirmed account loss versus narrow errors; source observation/expiry versus enforcement; and direct accept versus voluntary prejoin.
- Eleven existing specification files changed; desktop/accessibility, messaging and terminology remain byte-identical. The six review/quality inputs below were unchanged across this remediation. All writes stayed in the assigned paths. No Git operation, additional agent, VM/service startup, build, native/server/performance test, repository creation or publication occurred; no VM was started to hibernate.

## Candidate hashes

Repository-relative paths, SHA-256 of exact draft bytes for independent re-review. The report excludes its own hash to avoid self-reference. These are not commit-bound acceptance hashes.

| File | SHA-256 |
| --- | --- |
| `docs/compatibility/baselines-and-sources.md` | `370f2c0891697fcfc0b0f974e36b70e5184d2f425c66860337d91ecb02479fed` |
| `docs/compatibility/feature-inventory.md` | `3a50dc0a253a265156ad298db5776572c79e698471481f33ceaca7ae419d6f62` |
| `docs/compatibility/README.md` | `97083bfc7f05fdcbfea484403e807919b3d02859dfcf2f82f8bc76de98779d16` |
| `docs/compatibility/test-matrix.md` | `ea8392d039c8eb416e218ecbacdd8c89e0dcbc26c6f01c345fa3f76b0260c0fa` |
| `docs/product/accounts.md` | `d03fe9fc1888c67da1da60b094c03a27670ef0919a04f9f216636e3bf1908607` |
| `docs/product/calls.md` | `d0bf5bd5013a1764b4581bc8fd6b3daf4cc31a203f49d0f7bc4fb2f95d52a389` |
| `docs/product/conversations.md` | `1f746e087e824069c2ad6880e83e2ce11479de6451b7112d456e2b5ea7c08f95` |
| `docs/product/desktop-and-accessibility.md` | `87dc4d6b2374dcad10900c95dd2b298a9daaec2e663abf00b7afdf5bb24f140f` |
| `docs/product/files-and-content.md` | `4a6e41f1118c435dfe11465b98a9f3ab2a8abdde7cd4b88219d6001a27968390` |
| `docs/product/journeys-and-states.md` | `44b8f669059379d61c9dea9558bc22146dccb3a51080630fe6b325cd2b45ede2` |
| `docs/product/messaging.md` | `eee59ace54a6105f5729dfbc3aff6b72ff7133af26b061ce4098cd5f76182a87` |
| `docs/product/moderation-and-integrations.md` | `12fd3c4c583aae873bb9b263b2fd6095d96e10211eb5035944a4165b2c683a74` |
| `docs/product/README.md` | `fa6000d435af01a208a92005404b7ed330d29ee54aad77ce9ffc27fd5b4ebe86` |
| `docs/product/terminology.md` | `bbc98a617cff5e317d9bd14d0423f8a4ce580ebd3b8f095e14d4ec4859e0e99f` |

## Consulted review and quality hashes

| Input | SHA-256 |
| --- | --- |
| `docs/reviews/product-spec-review.md` | `f37f8c014c9a88e9d2fa3ee871b001b8f70d2eeb7b2816d6dee1eede0363e3f3` |
| `docs/reviews/quality-spec-review.md` | `26c8b5e69c13abcaf595afef9e16a341301b286c943a5606279e239f3cee88f7` |
| `docs/reviews/quality-remediation-r1.md` | `d4c55586c106ab4bc1048cc9fdbb2161333d21a9adea5e6df8378dac131fa946` |
| `docs/reviews/quality-spec-review-r2.md` | `9f64f886fbc289998e357c38f3db71e58ca0886518547cc691fdd204380750b2` |
| `docs/quality/security.md` | `4b7506eb4a3ac318752d68548db5d817bd9811ace86de7fbc87250c214ec1691` |
| `docs/quality/verification.md` | `e4c60ce8e61896003a340e69e073eb6b22acdab3dbd2022a94547d29d5b8561a` |

