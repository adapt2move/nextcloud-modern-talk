# Disposable Nextcloud/Talk verification lab

This contract defines disposable verification environments, not client support. Use the [backend lab guide](../backend-lab.md) for commands; server provisioning and API fixtures do not qualify native product behavior.

## Q.LAB-01 — Published candidates and immutable inputs

The Docker Official Images manifest lists `34.0.5-apache` and `35.0.2-apache`, including amd64 and arm64 variants. Both entries point to the Nextcloud Docker source revision `a1af1a30067045df8f258089cce4f31e50848ed4`. [Official image manifest](https://raw.githubusercontent.com/docker-library/official-images/master/library/nextcloud)

| Profile | Nextcloud image tag | Talk release | Reviewed Talk source commit |
| --- | --- | --- | --- |
| `NC34-CORE` | `docker.io/library/nextcloud:34.0.5-apache` | `24.0.6` | `17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79` |
| `NC35-CORE` | `docker.io/library/nextcloud:35.0.2-apache` | `25.0.6` | `f35255ab86f31e8ee894641f6970a09ce4b90927` |

The reviewed app manifests constrain Talk24 to NC34 and Talk25 to NC35. [Talk24 app manifest](https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/appinfo/info.xml), [Talk25 app manifest](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/appinfo/info.xml)

Registry-reported digests at research time:

| Image | Manifest-index digest | Linux amd64 manifest digest |
| --- | --- | --- |
| NC34 | `sha256:6d69b9052afab060afe8d004e9b00820f95a040b42db967b31eef9a48661e8bc` | `sha256:ec9d2572e94dd5eb2a2cbaff010cb1f1448bdf167f60597b9adcc89f601bdff1` |
| NC35 | `sha256:a588aa5353703b44106a6129adaaf7bf22521b7a4754f9f529b3228d6aa390c5` | `sha256:74c22796c8ff5537dd564394b47951cb90e4aa5e0f11467d062c539bb036bc35` |

Sources: [NC34 registry metadata](https://hub.docker.com/v2/namespaces/library/repositories/nextcloud/tags/34.0.5-apache), [NC35 registry metadata](https://hub.docker.com/v2/namespaces/library/repositories/nextcloud/tags/35.0.2-apache). Pin execution to an observed digest and explicit platform; record the resolved platform manifest. These are metadata observations, not locally verified image-content hashes. If the pin is unavailable or changed, stop and document a new candidate rather than substitute a floating tag.

Install the packaged Talk release, not an unbuilt source archive or a floating app-store selection. The distribution repository reports these assets and checksums:

| Package | Published SHA-256 |
| --- | --- |
| [spreed-v24.0.6.tar.gz](https://github.com/nextcloud-releases/spreed/releases/download/v24.0.6/spreed-v24.0.6.tar.gz) | `4a8d5cf52ed42223ea9937860f70c2e9dca1df8bfd5f75b871ae7569d2106110` |
| [spreed-v25.0.6.tar.gz](https://github.com/nextcloud-releases/spreed/releases/download/v25.0.6/spreed-v25.0.6.tar.gz) | `2b3e3eae42d9bf8ba8da2ab244d16347a3cfacbb8bff346bd99cf32cd6f9c65e` |

Sources: [Talk24 distribution metadata](https://api.github.com/repos/nextcloud-releases/spreed/releases/tags/v24.0.6), [Talk25 distribution metadata](https://api.github.com/repos/nextcloud-releases/spreed/releases/tags/v25.0.6). Verify the downloaded bytes and Nextcloud app integrity/signature at provisioning time. Record observed installed versions; do not assume source commit, package and running instance match merely because their labels agree. Disable automatic version changes during a run.

## Q.LAB-02 — Isolation, secrets and network scope

Use a disposable Linux Docker host for server tests. Use unique run labels, isolated container networks and separate disposable volumes for each server family. No production data, production credentials, host home-directory mounts, privileged containers or container access to the Docker socket. Do not reuse a newer database with an older image.

Generate independent cryptographically random ephemeral admin, test-user, app-password, database and optional TURN/signaling secrets. Deliver through supported secret-file/stdin mechanisms with restrictive permissions, outside the repository and normal workspace sync. Never put values in commands, shell tracing, committed configuration, screenshots or reports. Avoid dumping environment/configuration/container-inspection output. Redact service logs before retaining them; raw crash/media traces remain private until checked. Treat the disposable server's data volume, browser profile and macOS test Keychain items as secret-bearing artifacts too.

For `CORE`, bind HTTP/TLS only to VM loopback and use the session's actual local port forwarding; record the mapping without private host paths. Do not publish unspecified addresses, wildcard trusted domains or database ports. Verify host firewall, IPv4/IPv6 reachability and container runtime settings rather than assuming a localhost bind is sufficient in every Docker configuration. [Docker port publishing](https://docs.docker.com/engine/network/port-publishing/)

TLS verification stays enabled for the client. Use a temporary lab trust root/certificate or a controlled HTTPS endpoint, with trust setup/removal recorded. A labelled HTTP-loopback development smoke test is narrower evidence and cannot pass production TLS/auth acceptance. Extra media UDP/TCP ports are enabled only in the named media profile with least-necessary ranges, test-peer restrictions, expiry and cleanup; no anonymous public TURN relay.

The explicitly authorized [development transport](../backend-lab.md#reach-a-remote-lab) retains an unpublished internal Docker bridge and uses the assigned dev-box SSH identity/verified host key to forward one workstation IPv4-loopback port directly to its owned container. This equivalent authenticated transport has no VM HTTP listener or public inbound port. It avoids Docker 29's suppressed publication on an internal network. HTTP is confined to that isolated bridge and workstation loopback, with the inter-machine hop inside SSH; the CLI requires an explicit development-HTTP opt-in. No production TLS exemption or trust change follows. Both the local access helper and VM lab must be stopped before hibernation.

## Q.LAB-03 — Provisioning and functional profile

1. Record run ID, candidate commit/artifact, time budget, VM/runtime versions, image/platform digests and planned tests. Keep all generated secrets outside this manifest.
2. Create fresh `NC34-CORE` and `NC35-CORE` installations from the pinned inputs. The small functional profile may use the image's bundled SQLite; record its actual version. This is not a production scaling benchmark. Any PostgreSQL/MariaDB/Redis/reverse-proxy extension needs its own compatible version, image digest and configuration in the manifest. [Nextcloud Docker configuration](https://github.com/nextcloud/docker)
3. Install the exact Talk package, verify integrity, and collect redacted installed versions/capabilities/configuration plus health/maintenance state. Provision synthetic administrator, moderator, regular and restricted users, invitation-source groups and guests. Enable any required test-only 2FA provider with its exact version recorded.
4. Seed the product matrix's conversations and deterministic messages/files. Record fixture seed/hash and permission/preset settings. Exercise login, room/chat/files/guest workflows through the real client and API as appropriate, following [verification groups](verification.md#qver-05--mandatory-cross-cutting-scenarios).
5. Change capabilities/configuration/permissions through supported administration paths where available; repeat denied, absent and restored cases. Include account credential rejection during active work and non-hash policy changes during uninterrupted calls, plus control-connection loss while media persists when the media profile is provisioned ([V.AUTH.LOSS](verification.md#vauthloss--confirmed-account-authorization-loss), [V.POLICY.FRESH](verification.md#vpolicyfresh--ongoing-policy-freshness)). Keep labelled synthetic fixtures for states that cannot be produced safely on a real server. Separate server observations from contract-test results.
6. Export only redacted reproducibility metadata, results and fixture-generation instructions. Record cleanup separately before declaring the run complete.

Switch isolated profiles to test NC34/NC35 behavior; this is not an in-place database downgrade. `status.php`, an OCS response or a room join response proves only that particular API observation. It does not prove media exchange, device behavior, full feature parity or native-client performance.

## Q.LAB-04 — Optional HPB/TURN/media profile

Create `NC34-MEDIA` and `NC35-MEDIA` profiles when testing claimed media behavior. The reviewed Talk installation guide documents the high-performance backend and the `ghcr.io/nextcloud-releases/aio-talk` image family containing signaling/media/TURN components. Its floating `latest` example is discovery guidance, not an acceptable test pin. Resolve a concrete manifest digest/platform, component versions and configuration before use; HPB/TURN pins are **not selected or verified by this authoring phase**. [Pinned Talk installation guide](https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/docs/quick-install.md)

Record signaling, media gateway and TURN versions; TLS endpoints; permitted ports; NAT/firewall shape; relay-only versus direct connectivity; codecs; and peer-client versions. Use two real endpoints for direct calls and four for group media, including the native macOS candidate and a pinned official Talk client as an interoperability peer. Cover internal signaling separately when claimed. A port-forwarded HTTP connection does not establish UDP media reachability.

Verify audible bidirectional audio, visible moving video, screen-content delivery, mute/camera intent, device loss, reconnect and teardown. Record actual RTP/engine statistics and endpoint observations without real conversation data or exposed ICE addresses. A forced TURN run must confirm relay selection; reachable TURN credentials or a successful signaling connection alone are insufficient. Check lobby, recording consent and E2EE against the exact configuration.

Recording/transcription/SIP/federation require additional explicit profiles, dependencies and peers when claimed. Until configured and exercised, label them `not run` or `unsupported by client` as appropriate. Media-profile setup is optional for the research/bootstrap phase, mandatory for product claims it is intended to verify.

## Q.LAB-05 — Reproducibility evidence

Each run records: client commit/artifact; server image index/platform digests; Talk package SHA-256 and integrity result; installed versions; database/backend/peer versions; redacted capability/configuration snapshots; fixture hashes; network/port policy; timestamps; exact scenario IDs; expected/actual result; sanitized evidence links and limits. Use synthetic identifiers in published artifacts. Private endpoint mappings may remain in an access-controlled run record; public reports use logical endpoint labels.

Performance acceptance additionally requires the independently approved **pre-run** [workload manifest](verification.md), its immutable payload/input hashes and distinct activity/media profile ID. Record initial/reset/cache/page state, replay/validator versions, exact event/query schedule and publisher/subscriber/screen roles; observed event counts, residency and negotiated/delivered quality must validate against the frozen manifest. Attach native lifetime/peak collector qualification and per-run coverage evidence. Merely recording a seed or a selected codec afterward is insufficient. Invalid workload, incomplete accounting and a measured budget failure are separate results; none is a pass. See [V.PERF.WORKLOAD](verification.md#vperfworkload--frozen-and-validated-workload).

A second reviewer must be able to reconstruct the environment from pins, nonsecret configuration and fixture instructions, generating new secrets. Retain sanitized evidence and hashes, not a secret-bearing VM snapshot. Native/macOS and server results retain their separate environment identities.

## Q.LAB-06 — Cleanup and completion

Stop test clients/capture and revoke disposable app credentials where reachable; delete lab-owned browser sessions and vault entries. Remove only this run's containers, networks, volumes, temporary files/certificates, trust changes, port forwards and firewall exceptions. Verify no labelled process/listener/resource remains; never use a global destructive prune on a shared host.

Delete secret-bearing snapshots/artifacts created for the run or explicitly document and protect any retained artifact. If cleanup is interrupted, report residual resource IDs privately and the remaining cost/exposure; do not claim completion. Stop disposable compute after cleanup. Stopping a host alone is not credential/data deletion; preserve unrelated resources and snapshots.
