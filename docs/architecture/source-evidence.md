# Bootstrap primary-source evidence

Read-only source research on **2026-10-18**. No upstream code was executed or copied into the implementation. These observations support interface feasibility and identify qualification gaps; they do not certify integration, source timing, platform behavior or license clearance. The accepted [baseline/release history](../compatibility/baselines-and-sources.md) remains unchanged.

## A-SRC-01 — Rust and native UI

- [Official Rust 1.99.1 announcement](https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/) and [release index](https://blog.rust-lang.org/releases/) establish the selected toolchain release.
- Zed research commit `56934ec0560bce8929025497411de7f5322fb6ca`: [GPUI README](https://github.com/zed-industries/zed/blob/56934ec0560bce8929025497411de7f5322fb6ca/crates/gpui/README.md), [crate manifest](https://github.com/zed-industries/zed/blob/56934ec0560bce8929025497411de7f5322fb6ca/crates/gpui/Cargo.toml), [upstream toolchain](https://github.com/zed-industries/zed/blob/56934ec0560bce8929025497411de7f5322fb6ca/rust-toolchain.toml). The inspected GPUI manifest is version 0.2.2, Apache-2.0; upstream toolchain is 1.98.1, while README asks for current stable Rust. Neither is a pinned dependency of this workspace. Native Metal/platform setup and changing APIs require an actual integration choice later.

## A-SRC-02 — Both-family policy sources

Use the same immutable Talk research pins as the accepted specification: **T24** `17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79` and **T25** `f35255ab86f31e8ee894641f6970a09ce4b90927`. They are not the corresponding released-tag commits. Both require release-profile verification before compatibility claims.

| Source | Both-family evidence and conclusion |
| --- | --- |
| Capability emitter | [T24](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/lib/Capabilities.php), [T25](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Capabilities.php): call enabled, recording/consent, E2EE and grid fields are emitted; local feature/config lists define authority. The E2EE feature is conditional on external signaling. |
| Config accessors | [T24](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/lib/Config.php), [T25](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Config.php): actual E2EE/grid/consent accessors exist. Different config plumbing across families is not a cache/cluster timing guarantee. |
| Settings/hash | [T24](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/settings.md), [T25](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/settings.md): non-hash E2EE/grid settings require full periodic reads. Admin setting names are not new public read endpoints. |
| Room/current session | Conversation docs [T24](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/conversation.md), [T25](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/conversation.md); response definitions [T24](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/lib/ResponseDefinitions.php), [T25](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/ResponseDefinitions.php): effective permissions and call/consent/session data; response definitions include attributes. Room session ID is a precheck, not permanent session authority. |
| Call/session and consent | Call docs [T24](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/call.md), [T25](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/call.md); call controllers [T24](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/lib/Controller/CallController.php), [T25](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/CallController.php): call list/join/leave and required consent checks. T25 checks classified before applying consent; this does not permit recording. |
| Constants | [T24](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/constants.md), [T25](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/constants.md): permission bit and attribute interpretations; unknown values must not expand authority. |

Freshly retrieved source hashes (SHA-256) for the concrete mapping implementation files:

| File | T24 | T25 |
| --- | --- | --- |
| `lib/Capabilities.php` | `6b104ae22531f460773eaa718fa94a66eb5a4f137249ec321ca9556542e8934d` | `bf73117e6a2bcbbe10eac27d2f9c57feb3f013a3163d108b796f27fda76b7d18` |
| `lib/Config.php` | `3d2bbc1826e50e938a3ecb281dea55b13053d0705e9c4d21a3a161ec431e0c8b` | `50415256943a46c751309679b0aecded024039949bbb07be275e527822ba52cf` |
| `lib/ResponseDefinitions.php` | `fd851c90365b5e8ecf47d9d118a9497e3935a09f2443e385fb54449668f4b697` | `e488fc11463130f604e60af7fed423ec4a306a5b83e6f1e39a2e32c889bb3d39` |
| `lib/Controller/SignalingController.php` | `f4eaa13a70f05bf0cab8aa508893509577cf37604b1935c7db247b518228add1` | `43272bc88e486cb846ce66d000bd135053831b4a6d81785bc837fef32a0fdc09` |

These reads establish fields/routes, not `S=2 s`. PHP/config/database/proxy paths must be bounded and cache bypass tested across every deployed node. No arbitrary deployment has been qualified. Failed/missing evidence leaves the relevant native paths unsupported.

## A-SRC-03 — Signaling

[T24 signaling controller](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/lib/Controller/SignalingController.php), [T25 signaling controller](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Controller/SignalingController.php), and internal-signaling docs [T24](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/docs/internal-signaling.md)/[T25](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/internal-signaling.md) support the v3 settings and pull/send routes. Settings include mode/server, tickets/hello parameters and STUN/TURN data. The internal protocol prose has an incomplete section, so a running native implementation cannot be inferred from those docs.

The separately pinned [HPB protocol](https://github.com/strukturag/nextcloud-spreed-signaling/blob/c3817ac2402ef764b8d4e6dc03ac7e60ffab49d3/docs/standalone-signaling-api-v1.md) documents hello, room, session expiry, resume and federation interruption. This pin is source research, not a selected deployed HPB binary. None of these messages supplies a complete, bounded positive authority for all Nextcloud policy fields. They can invalidate known continuity while full source reads establish current policy.

## A-SRC-04 — Account probe

Use released NC34 commit `77e3cab12d2215bf94d6de67f23b98ed7bad236e` and NC35 commit `ba900af933e61ffa005315b09d6454bfbbcbcc81` here, separately from Talk research branches:

- User routes [34](https://github.com/nextcloud/server/blob/77e3cab12d2215bf94d6de67f23b98ed7bad236e/apps/provisioning_api/appinfo/routes.php)/[35](https://github.com/nextcloud/server/blob/ba900af933e61ffa005315b09d6454bfbbcbcc81/apps/provisioning_api/appinfo/routes.php) map `/cloud/user` to the current user controller [34](https://github.com/nextcloud/server/blob/77e3cab12d2215bf94d6de67f23b98ed7bad236e/apps/provisioning_api/lib/Controller/UsersController.php)/[35](https://github.com/nextcloud/server/blob/ba900af933e61ffa005315b09d6454bfbbcbcc81/apps/provisioning_api/lib/Controller/UsersController.php).
- OCS unauthorized code and response handling: [public OCS controller](https://github.com/nextcloud/server/blob/ba900af933e61ffa005315b09d6454bfbbcbcc81/lib/public/AppFramework/OCSController.php); v2 responses [34](https://github.com/nextcloud/server/blob/77e3cab12d2215bf94d6de67f23b98ed7bad236e/lib/private/AppFramework/OCS/V2Response.php)/[35](https://github.com/nextcloud/server/blob/ba900af933e61ffa005315b09d6454bfbbcbcc81/lib/private/AppFramework/OCS/V2Response.php); middleware [34](https://github.com/nextcloud/server/blob/77e3cab12d2215bf94d6de67f23b98ed7bad236e/lib/private/AppFramework/Middleware/OCSMiddleware.php)/[35](https://github.com/nextcloud/server/blob/ba900af933e61ffa005315b09d6454bfbbcbcc81/lib/private/AppFramework/Middleware/OCSMiddleware.php). These motivate the narrowly scoped 997/401 classifier; actual endpoint envelopes remain live-test prerequisites.
- Core capabilities controllers [34](https://github.com/nextcloud/server/blob/77e3cab12d2215bf94d6de67f23b98ed7bad236e/core/Controller/OCSController.php)/[35](https://github.com/nextcloud/server/blob/ba900af933e61ffa005315b09d6454bfbbcbcc81/core/Controller/OCSController.php) expose `/cloud/capabilities`, allow public responses and set ETags. Consequently neither a capabilities 200 nor a matching ETag proves valid account credentials or fresh protected policy.

Version-specific Login Flow and app-password deletion sources remain [S17](../compatibility/baselines-and-sources.md#protocol-sources). The controller attributes and actual route files above are the routing evidence used.

## A-SRC-05 — Native measurement

Apple XNU source research pin `f6217f891ac0bb64f3d375211650a4c1ff8ca1ea`: [task memory fields](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/mach/task_info.h) and [resource usage structures](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/sys/resource.h). Separate footprint and lifetime-peak fields exist. Their declarations alone do not establish target-OS rights, structure revisions, correctness or post-exit availability; see the [qualification plan](measurement.md).

Keychain/platform evidence and SQLite durability sources are linked in [ADR-003](decisions/003-credential-lifecycle.md). Native process/device termination, filesystem durability, post-crash writer settlement and clock/sleep behavior remain unproven. No secret-bearing runtime artifacts or downloaded upstream implementation files were added by this research.
