# Disposable NextCloud/Talk lab requirements

Use the [backend lab guide](../backend-lab.md) and
[harness reference](../../tests/lab/README.md) for runnable commands. These
requirements govern isolated tests; provisioning is not proof of native-client support.

## Q.LAB-01 — Immutable inputs

[tests/lab/pins.json](../../tests/lab/pins.json) owns the executable image digests,
Talk package checksums and installed version expectations for NC34.0.5/Talk24.0.6
and NC35.0.2/Talk25.0.6. Verify downloaded bytes, app integrity and actual installed
versions. Record the resolved platform manifest; do not substitute floating tags
or branch archives when a pin is unavailable. Updating pins requires source review
and affected tests. Never downgrade a database in place.

## Q.LAB-02 — Isolation and secrets

Use disposable environments, unique run labels, isolated networks and separate volumes
for each profile. Use synthetic content and independently generated ephemeral secrets
through private files/stdin outside repository sync. No production accounts, host-home
mounts, privileged containers or Docker-socket access from containers. Keep raw logs,
media traces, secret-bearing browser profiles and service configuration private.

Keep HTTP on loopback or the authenticated SSH bridge described in the backend guide;
verify actual firewall and IPv4/IPv6 exposure. The lab's explicit development-HTTP opt-in
never changes production TLS validation. Extra media ports need a scoped profile,
restricted peers and cleanup; never create an anonymous public TURN relay.

## Q.LAB-03 — Functional coverage

Record the candidate, pins, environment, time budget and planned cases before running.
Seed synthetic users, roles, rooms, messages and files with reproducible fixture hashes.
Exercise positive, denied, missing-capability, uncertain-write, cancellation and recovery
cases from the [test scenarios](../compatibility/test-matrix.md) and
[verification groups](verification.md#qver-05--mandatory-cross-cutting-scenarios).
Label injected faults separately from actual server behavior. A successful API response
does not establish capture, media exchange or native accessibility.

## Q.LAB-04 — Media and optional backends

Pin signaling, Janus, TURN and peer-client versions/configuration for each topology;
see [media checks](../../tools/media/README.md). Internal signaling, HPB and forced
relay are distinct profiles. Confirm actual bidirectional audio/video, selected relay,
screen delivery, device loss, reconnect, policy enforcement and local shutdown.
Port-forwarded HTTP does not establish UDP connectivity. Recording, transcription,
SIP and federation need their own configured peers and checks. Native macOS devices,
Keychain, permissions and VoiceOver require a Mac; Linux cannot qualify them.

## Q.LAB-05 — Reproducibility

Retain sanitized candidate/artifact identity, server/package/backend pins, fixture hashes,
capability/policy observations, commands, expected/actual outcomes and limits in the
issue, PR or CI artifacts. A second operator must be able to reconstruct the run with
new secrets. Performance acceptance additionally requires the frozen workload and
complete native collection in the [measurement contract](../architecture/measurement.md);
API smoke checks and sampled RSS cannot establish those budgets.

## Q.LAB-06 — Cleanup

Stop clients and captures, revoke disposable credentials where reachable, and remove
only run-owned containers, networks, volumes, browser/vault items, temporary files,
certificates, trust changes and forwards. Verify no owned service/listener remains.
Delete secret-bearing snapshots and stop disposable compute. Do not use global prune
on shared hosts. Report interrupted cleanup and residual resources honestly.
