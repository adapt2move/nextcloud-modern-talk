# Compatibility contract

Status: **Planned; no native interoperability runs recorded**. Target families are Nextcloud 34.x/Talk 24.x and Nextcloud 35.x/Talk 25.x. The [verified research baseline](baselines-and-sources.md) establishes that the source and published versions exist; it does not certify this client against them.

Read the [feature inventory](feature-inventory.md) for every planned feature and the [test matrix](test-matrix.md) for required evidence. Product behavior is canonical in [product/README.md](../product/README.md).

## COMP-001

**Resolve availability using every relevant gate.** An action is available only when its client implementation, correct-scoped API capability, enabled configuration/backend, conversation type/attributes, effective permissions, session state, and any OS authorization all permit it. A version, role label, preset name, mock fixture, or installed app alone never unlocks it.

Acceptance: a newer version with a disabled feature denies it; an older supported patch with a valid backport and implemented compatible contract may enable it. A supported server API with an unimplemented native feature shows the client limitation. A temporarily unavailable backend is not reported as permanently unsupported.

## COMP-002

**Scope capabilities to the account and conversation host.** Fetch the local account's OCS `spreed` capabilities, including `features`, `features-local`, `config`, and `config-local`. The local lists identify which feature/config values belong to the user's local server. For federated rooms, resolve other values against the conversation host's capability response via the supported proxy contract. Also verify that the local proxy supports the endpoint's federation level. Do not union both servers' features or require both to agree on every local-only setting.

Acceptance: a local attachment preference does not come from the remote host; a remote call policy does not come from the local account's unrelated default. Capabilities cannot leak across accounts, hosts, guest sessions, or room tokens. Missing host evidence leaves the affected action unknown/blocked until resolved. [Capability scoping evidence](baselines-and-sources.md#protocol-sources).

## COMP-003

**Refresh capabilities and policy with bounded freshness.** A changed `X-Nextcloud-Talk-Hash` invalidates the relevant local capability/signaling snapshot; a changed `X-Nextcloud-Talk-Proxy-Hash` invalidates the corresponding host snapshot. Recompute actions and pending form/join validity. Reauthenticate/reconnect triggers a refresh as appropriate. Periodic/full room reconciliation is required because incremental room updates do not report every removal.

Hash invalidation is only one trigger. [Q.SEC-10](../quality/security.md#required-bootstrap-policy-decision) is authoritative for ongoing calls as well as joins: scope fresh/denied/unknown/stale decisions to account, host, room and authorization/call generation, and refresh every required property through a proven source. In the pinned settings, E2EE and grid enforcement can change without a new hash. A matching hash, heartbeat, working RTP or unrelated response cannot renew those values. Before `G.BOOT`, approve source/propagation assumptions and numeric freshness, refresh, request, enforcement and device-release bounds; unfilled bounds or an unobservable required source cannot support an enforcement claim.

Acceptance: disabling calls, changing forced presets/consent or revoking permission during a dialog invalidates obsolete submission on observation; uninterrupted-call non-hash changes are detected/enforced or safely suspended within the approved bounds. Control loss remains visible even with live media; failed/partial refresh cannot reset expiry, and delayed/old-generation responses cannot renew authority. Unknown/stale evidence enables no new join/publication expansion; existing grants expire under Q.SEC-10, with local leave when membership/required consent/encryption cannot be established. Fresh state alone never restarts capture. An update for one host does not clear unrelated accounts. [CT-004/015/018](test-matrix.md#ct-004) and [V.POLICY.FRESH](../quality/verification.md#vpolicyfresh--ongoing-policy-freshness) require source/expiry/enforcement evidence, including sleep/wake.

## COMP-004

**Preserve unknown data and conservative security behavior.** Decode known types, flags, roles, object associations, and capabilities while retaining unknown raw values. An absent optional field uses a default only if the applicable protocol defines one. Absent required evidence, malformed values, and unknown restrictive semantics must not become “allowed”. Updates must not erase unknown attributes.

Acceptance: future room type/attribute fixtures do not become editable groups. Unknown rich content has a safe fallback. Unknown recording/E2EE/classified policy blocks the relevant operation. Ordinary readable content need not be hidden solely because an unrelated optional feature is unknown; the restriction must correspond to the unresolved risk.

## COMP-005

**Use supported protocol families deliberately.** The reviewed baselines document conversation/call API v4 and signaling v3; removed API generations are not a compatibility fallback. Endpoint versions differ by feature: chat, presets, polls, and other APIs must use their own verified contracts. Request/validate the expected OCS/JSON structure and distinguish HTTP/OCS errors. Session/cookie behavior for joining rooms/calls must be implemented and tested; an app password alone is not proof that a media session exists.

Acceptance: an unsupported endpoint/version shows a meaningful limitation or refreshes evidence; it never probes mutation variants blindly. A room `404` does not necessarily mean account revocation. Request identity and authorization follow installation base paths, account scope, and the security contract.

## COMP-006

**Handle patches, backports, and unknown deployments.** Treat exact tested release tuples as evidence, not a whitelist of version strings that substitute for capabilities. Test the current supported patches plus the earliest patch a release claims to support. Re-test changes that affect protocol or policy. A version outside the target families is explicitly unverified; allow only understood safe behavior with conservative gates and no compatibility guarantee. Misreported version and capability data are an error/diagnostic condition.

Acceptance: Talk 24.0.1 backports are not hidden merely because documentation groups similar features under Talk 25. A 34/25 or 35/24 pairing is not claimed supported; the app manifests restrict the reviewed pairing. A vendor backport requires the actual compatible feature contract and tests, not renaming the version.

## COMP-007

**Separate server support, client support, and backend readiness.** Capabilities may depend on installed services, admin configuration, account groups, a room's origin, or its current policy. HPB/external signaling, TURN, recording, transcription, SIP, bridges, and provider apps need named deployment evidence. A server advertising E2EE only establishes a prerequisite; actual native media encryption must be proven.

Acceptance: a successful chat test never implies working calls/recording/federation. Both absent capability and advertised-but-unavailable backend cases are tested. Browser handoff is identified as such and never counted as native media parity. The external iframe-service contract remains outside native embedding scope.

## COMP-008

**Record compatibility evidence before making claims.** Every verification run names client revision, OS/architecture, exact server/Talk versions or commits, artifact digest, local/host capabilities, role/permissions, room attributes, backend versions/configuration, test IDs, result, and redacted artifacts. Release claims are limited to the passing scope. Research-only, mock, contract-fixture, native adapter, and live integration results are distinguishable.

Acceptance: all current inventory entries remain **Planned** until evidence exists. A skipped case records a reason and preserves the unsupported/unverified limitation; it cannot be counted as passing. Source citations do not become test reports.
