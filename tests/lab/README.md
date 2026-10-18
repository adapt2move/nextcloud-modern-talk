# Disposable backend fixtures

This bootstrap harness installs two **released** Nextcloud/Talk pairs and exercises a small API smoke suite. It implements no native product feature. All 140 product inventory entries remain Planned. The accepted [lab contract](../../docs/quality/test-servers.md) and [compatibility matrix](../../docs/compatibility/test-matrix.md) remain the broader acceptance requirements.

| Profile | Server | Talk package | Runtime |
| --- | --- | --- | --- |
| `r34` | NC 34.0.5 Apache | 24.0.6 | Linux amd64, bundled SQLite |
| `r35` | NC 35.0.2 Apache | 25.0.6 | Linux amd64, bundled SQLite |

[pins.json](pins.json) pins the **platform image manifest**, packaged application checksum and release-source identity. Tags/index digests are provenance references, not floating runtime inputs. A missing image, mismatched checksum, wrong installed version or failed app-integrity check fails the profile; no replacement version is selected automatically.

## Provision, check, clean up

Prerequisites: a disposable Linux amd64 VM with Python 3.11+ and Docker Engine 28+; Internet access for the pinned image/package downloads. Runtime secrets live in restricted `/run` directories, which should be tmpfs. Allow approximately 4 GiB RAM, 10 GiB free disk and a 30-minute execution budget. The two profiles run sequentially; this is a functional fixture, not a production deployment or performance benchmark.

Use the project/session's existing agent-dev-box environment. Set `LAB_SESSION` to that real session ID in your terminal; it is an identifier, not a credential. Do not use a new child session. From the bound workspace containing this repository:

```sh
agent-dev-box up --session "$LAB_SESSION" --size small
agent-dev-box sync --session "$LAB_SESSION"
agent-dev-box exec --session "$LAB_SESSION" -- bash -lc \
  'cd nextcloud-modern-talk && sh tests/lab/reprovision.sh --output /tmp/nmt-lab-evidence'
```

If the session is bound to the repository itself, omit the remote `cd nextcloud-modern-talk`. `up` rejects a different workspace for an already bound session; retain the session's binding. For a previously hibernated session, use `resume`. Source changes must be local and synced; sync excludes `.git`, does not delete stale remote files and does not copy remote evidence back.

The output directory must be empty and outside the repository. Optional flags: `--profile r34`, `--profile r35`, `--budget-seconds 1800`. Each run creates new independent credentials and data. Rerunning is reprovisioning, not an upgrade/downgrade or an attempt to reuse a database. No credential environment variables or production account are required. This suite is manual/opt-in and must not depend on repository secrets in untrusted pull-request CI.

The Python standard-library harness is an equivalent to Compose with no external Python dependency. It creates precisely labelled containers, networks and volumes; publishes **no host ports**; uses separate internal Docker networks reachable directly by the VM; and makes no dev-box port forward or firewall change. Docker 28+ is the qualified minimum. The fixture never requests port publishing; the explicit internal network is inspected before HTTP access. [Docker's port-publishing guidance](https://docs.docker.com/engine/network/port-publishing/)

This profile deliberately uses HTTP over the isolated Docker bridge. Requests use the exact trusted Host `127.0.0.1`; the dynamic bridge address is runtime-only. This differs from Q.LAB-02’s forwarded-loopback example: there is no host listener or forward to expose. It does **not** exercise TLS, certificate validation, browser Login Flow v2, 2FA/SSO or client credential storage. Container memory/CPU/process caps bound the disposable fixture. SQLite has no separate database password; admin/user passwords are random per run. The administrator password enters through a restricted secret-file mount; user passwords through `docker exec` stdin. No password appears in Docker arguments or environment metadata. API credentials/cookies and room/session identifiers are omitted from public evidence. The credentials are synthetic server passwords used for Basic-auth probes, **not browser-issued app passwords**.

## Checked scenarios

Calls use `/ocs/v2.php` and JSON OCS envelopes. `LAB-*` IDs describe only these probes and partial matrix coverage, never a pass of an entire `CT-*` scenario.

| ID | Assertions / actual routes attempted | Related acceptance scope |
| --- | --- | --- |
| LAB-001 | Pull exact platform digest; inspect image identity; download and hash exact Talk archive | CT-001 subset |
| LAB-002 | Assert zero published ports, exactly one network and its internal flag; independently inspect VM firewall/listeners for the recorded run | Q.LAB-02 subset |
| LAB-003 | `/status.php`; `occ config:app:get spreed installed_version`; `occ integrity:check-app`; PHP/SQLite version | CT-001 subset |
| LAB-004 | `GET /cloud/user` identifies four separate synthetic users; anonymous `GET /apps/spreed/api/v4/room` is 401 | CT-002 API-auth subset only |
| LAB-005 | `GET /cloud/capabilities`; record advertised Talk features and selected nonsecret configuration fields | CT-001/004 discovery subset |
| LAB-006 | `GET /apps/spreed/api/v1/presets/room` as regular user and admin; assert the pinned released-default sets | CT-005 discovery subset |
| LAB-007 | `POST /apps/spreed/api/v4/room`; invite via `/{token}/participants`; join via `/{token}/participants/active` as owner and member | CT-005/006 subset |
| LAB-008 | `POST` then `GET /apps/spreed/api/v1/chat/{token}`; member sees same message ID, Unicode body and author | CT-007 subset |
| LAB-009 | Authenticated outsider cannot retrieve private room or history (404); outsider's room list excludes it | CT-002/007 server isolation subset |
| LAB-010 | `POST /apps/spreed/api/v1/poll/{token}`; member votes at `/{pollId}`; ordinary voter cannot `DELETE` (403); open state remains; author closes; member reads retained result; late vote fails (400) | CT-010 backend subset; **no native Close action** |
| LAB-011 | Only when `channel` is returned: ordinary group call-control join/leave succeeds with no media flags, then create channel preset, check its attribute, owner joins conversation, owner call join at `/apps/spreed/api/v4/call/{token}` fails (403) | CT-005 selected invariant; **no media call** |

Preset expectation differences are assertions about these fresh server defaults. A product implementation must use actual current capability/configuration/permission gates; these fixture expectations are not a major-version feature switch. The extra channel assertion is selected by the returned preset list. Its absence is explicitly not applicable. Poll GET/vote strictly expect HTTP 200, matching both released controllers and the observed response; upstream `docs/poll.md` lists 201 for these operations. Semantic vote/state checks are additional. The call positive control is HTTP state only: `flags=1` advertises neither audio nor video, and no WebRTC peer or capture exists.

Official release-specific protocol sources: [Talk24 conversation](https://github.com/nextcloud/spreed/blob/0d736588dcfb42becf0e5688e0847e35d5b5efa2/docs/conversation.md), [Talk25 conversation](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/docs/conversation.md), [Talk25 preset factory](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/lib/Service/RoomPresetFactory.php), [poll API prose](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/docs/poll.md), [Talk24 poll controller](https://github.com/nextcloud/spreed/blob/0d736588dcfb42becf0e5688e0847e35d5b5efa2/lib/Controller/PollController.php), [Talk25 poll controller](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/lib/Controller/PollController.php), [chat API](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/docs/chat.md), [call controller](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/lib/Controller/CallController.php). Release package sources/checksums are recorded in the [accepted lab research](../../docs/quality/test-servers.md#qlab-01--published-candidates-and-immutable-inputs).

## Evidence and failure recovery

`report.json` records run timestamps, executing source-file SHA-256s, artifact pins and observed identities, operations/statuses, semantic assertions, errors, and cleanup. The VM copy has no Git history, so the manager must bind the hashes to the final source commit before using it as acceptance evidence. Copy only this allowlisted report after review. Never publish a full container inspection, environment/config dump, raw server log, `/run` directory, database, cookie jar or unreviewed snapshot.

Normal success, failure, SIGINT and SIGTERM attempt cleanup. The recovery state location is printed at startup; it contains resource identities, no credential values. After a killed runner or interrupted host connection, inspect the existing dev-box job before starting another. Recover with:

```sh
agent-dev-box exec --session "$LAB_SESSION" -- bash -lc \
  'cd nextcloud-modern-talk && sh tests/lab/cleanup.sh --state /run/nmtlab-RUN/state.json'
```

Replace the placeholder with that run's printed state location. Cleanup checks exact ownership labels, removes only its recorded containers/networks/volumes, checks for remaining run-labelled resources, then removes its runtime directory. It never invokes global prune. Failed cleanup retains the recovery record and fails the run. Downloaded packages and passwords are removed before hibernation; immutable public image cache layers may remain. No secret-bearing lab snapshot should be retained.

After inspecting cleanup and retrieving sanitized evidence:

```sh
agent-dev-box stats --session "$LAB_SESSION" --json
agent-dev-box hibernate --session "$LAB_SESSION"
agent-dev-box status --session "$LAB_SESSION"
```

Wait for `phase: hibernated`; record snapshot and estimated cost separately. Hibernation is not a substitute for data cleanup. If other work explicitly owns the same running VM, coordinate that lifecycle with its owner.

Browser authorization, revoked-credential/callback races, guest/file/federation workflows, most preset restrictions, Keychain fallback, GPUI, accessibility, WebRTC/audio/video/screen sharing, E2EE, HPB/TURN, recording, native performance and full parity remain **not tested here**. HPB/TURN artifacts are not pinned or provisioned by this harness. [Recorded execution evidence](../../docs/testing/README.md) states the actual completed subset and limitations.
