# Upstream references

Use [Cargo.toml](../../Cargo.toml), [Cargo.lock](../../Cargo.lock) and
[rust-toolchain.toml](../../rust-toolchain.toml) for the exact build inputs.
References below explain protocol and platform interfaces. They are not substitutes
for the [compatibility scenarios](../compatibility/test-matrix.md), actual server
capabilities or platform tests.

## A-SRC-01 — Rust and native UI

The desktop uses the pinned GPUI 0.2.2 crate. Its responsibilities and renderer
boundary are described in [native UI architecture](decisions/001-native-ui.md).
The local [GPUI guidance](../../.agents/skills/gpui-guidelines/SKILL.md) links the
upstream APIs used for lifetime, rendering, focus and input. Platform compilation
and native behavior remain separate checks.

## A-SRC-02 — Both-family policy sources

The [supported baselines](../compatibility/baselines-and-sources.md) identify the
NextCloud and Talk versions used for compatibility. The API crate maintains its
[pinned OpenAPI input and operation inventory](../../crates/nextcloud-talk-api/README.md).
For server authority, consult the release-specific capability emitter, response
schemas and permission constants:

- Talk 24: [capabilities](https://github.com/nextcloud/spreed/blob/v24.0.6/lib/Capabilities.php),
  [response definitions](https://github.com/nextcloud/spreed/blob/v24.0.6/lib/ResponseDefinitions.php),
  [constants](https://github.com/nextcloud/spreed/blob/v24.0.6/docs/constants.md).
- Talk 25: [capabilities](https://github.com/nextcloud/spreed/blob/v25.0.6/lib/Capabilities.php),
  [response definitions](https://github.com/nextcloud/spreed/blob/v25.0.6/lib/ResponseDefinitions.php),
  [constants](https://github.com/nextcloud/spreed/blob/v25.0.6/docs/constants.md).

Capabilities describe supported operations; current room/session responses supply
permissions and participation. Neither a version number nor cached capabilities
grant publication authority. See the [policy lifecycle](decisions/004-policy-media-lifecycle.md).

## A-SRC-03 — Signaling

The ordinary call profile uses the released Talk internal-signaling API:
[T24](https://github.com/nextcloud/spreed/blob/v24.0.6/docs/internal-signaling.md)
and [T25](https://github.com/nextcloud/spreed/blob/v25.0.6/docs/internal-signaling.md).
The corresponding call contracts are
[T24](https://github.com/nextcloud/spreed/blob/v24.0.6/docs/call.md) and
[T25](https://github.com/nextcloud/spreed/blob/v25.0.6/docs/call.md).

The [HPB protocol](https://github.com/strukturag/nextcloud-spreed-signaling/blob/c3817ac2402ef764b8d4e6dc03ac7e60ffab49d3/docs/standalone-signaling-api-v1.md)
is a reference for a different signaling profile. Its existence does not establish
HPB, federation or end-to-end-encrypted media support in this alpha.

## A-SRC-04 — Account probe

The authoritative current-user probe is same-origin
`GET /ocs/v2.php/cloud/user?format=json`. Pinned released server routes are
[NC34](https://github.com/nextcloud/server/blob/77e3cab12d2215bf94d6de67f23b98ed7bad236e/apps/provisioning_api/appinfo/routes.php)
and [NC35](https://github.com/nextcloud/server/blob/ba900af933e61ffa005315b09d6454bfbbcbcc81/apps/provisioning_api/appinfo/routes.php).
The OCS v2 responses are defined in
[NC34](https://github.com/nextcloud/server/blob/77e3cab12d2215bf94d6de67f23b98ed7bad236e/lib/private/AppFramework/OCS/V2Response.php)
and [NC35](https://github.com/nextcloud/server/blob/ba900af933e61ffa005315b09d6454bfbbcbcc81/lib/private/AppFramework/OCS/V2Response.php).

The [credential contract](decisions/003-credential-lifecycle.md) defines the narrow
HTTP 401 / OCS 997 rejection classifier. A public capabilities response is not
proof of authenticated account health. Browser Login Flow and app-password
lifetime references are indexed with the [protocol sources](../compatibility/baselines-and-sources.md#protocol-sources).

## A-SRC-05 — Native measurement

Apple's pinned [task memory fields](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/mach/task_info.h)
and [resource usage structures](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/sys/resource.h)
distinguish physical footprint from resident memory and sampled peaks.
The [measurement contract](measurement.md) explains required collector validation;
these declarations are not a measured performance result.

## A-SRC-06 — Native media research and shipping inputs

The shipping transport is [`webrtc =0.21.1`](https://docs.rs/crate/webrtc/0.21.1/source/), an async runtime over `rtc =0.21.1`. Its registry archive SHA-256 is `b621e736302bef784948cc8d8099fd0a8fdb5757693c80a344ffbe38c49387d2`. The released `play-from-disk-vpx`, `save-to-disk-vpx` and data-channel examples define the API used by the engine; older 0.14 examples do not apply. The [lockfile](../../Cargo.lock) pins the complete resolution, including the safe GStreamer codec bindings.

[Native media architecture](decisions/005-native-media.md) describes ownership, limitations and packaging. Rust dependency and native codec notices accompany packages; a separate libwebrtc SDK is no longer a shipping input.

The older [media source index](evidence/native-media-sources.json) preserves
attributed upstream references for codecs, transform APIs and Talk framing.
It is research provenance, not the shipping engine identity or an SBOM. Application
queue bounds do not establish codec, framework, GPU or whole-process memory
bounds; see [allocation requirements](media-allocation.md).
