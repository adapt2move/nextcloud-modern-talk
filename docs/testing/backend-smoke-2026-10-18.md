# Released-backend smoke evidence — 2026-10-18

**Result: pass for the bounded backend fixture on both pinned pairs.** This is author-executed API evidence for the bootstrap, pending independent review and commit binding. It is not a G.BOOT acceptance decision, native-client integration result, performance benchmark or feature-completion claim. All 140 product features remain Planned.

## Run and artifact identity

Successful run: `nmtlab-20261018t161202z-a1c36327`, **16:12:02–16:13:22 UTC**. [Machine-readable evidence](evidence/backend-smoke.json) records every operation, assertion, artifact identity and cleanup result. SHA-256 of that report: `6b85d61c3a1957b67a30fb710a46f21d96cd2f9dd0d7f43b526df13a4221d84d`.

The source is an uncommitted bootstrap following frozen specification `adfb44a2aab2e29ba6574c9dacc09475c534bccf`; `.git` is excluded from VM sync. The report contains the exact SHA-256s of all five executed harness/support files. In particular, `tests/lab/lab.py` was `ce5c9eaac74c701795587bbceb67bb885e2b93c74dca02d8004bddda0ca86678`. The project manager must bind those hashes to the eventual source commit; no commit or publication was performed by this contributor.

Environment: one small CPX22 development VM; Linux x86_64, kernel 7.0.0-30-generic, Docker 29.1.3, Python 3.14.4. Each server used its image's PHP **8.5.11** and SQLite **3.46.1**. Profiles were fresh, separate and sequential. Fixed synthetic actors/content are defined in the hashed harness; credentials and room/session identifiers are deliberately random and excluded from evidence.

| Executed profile | Observed server / internal version | Observed Talk | Downloaded Talk bytes | HTTP operations | Assertions |
| --- | --- | --- | --- | --- | --- |
| R34 / `r34` | 34.0.5 / 34.0.5.1 | 24.0.6 | 53,797,999 | 25 | 96 pass, 1 not applicable |
| R35 / `r35` | 35.0.2 / 35.0.2.2 | 25.0.6 | 54,265,379 | 30 | 111 pass |

Both pulled Linux amd64 manifest digests exactly matched [pins.json](../../tests/lab/pins.json), as did the SHA-256s computed over downloaded packaged Talk releases. The report retains full observed `RepoDigests` and image identities; tag existence alone is not the evidence. Installed app manifests matched 24.0.6 with NC min=max 34 and 25.0.6 with NC min=max 35. Signature files existed and `occ integrity:check-app spreed --output=json` returned exit 0 with no errors. Both installations reported installed, maintenance off and no database upgrade needed. No image/package substitution or app-store-selected version was used.

## Completed observations

The [scenario/route table](../../tests/lab/README.md#checked-scenarios) specifies the exact calls. The successful JSON records their actual HTTP/OCS statuses and semantic assertions.

- **LAB-001–005:** immutable inputs, installed versions and integrity, four distinct authenticated synthetic user identities, anonymous room-list rejection (401), capabilities and selected nonsecret configuration. Both advertised calls and conversation creation enabled, recording disabled and recording consent 0. This is Basic-auth testing with synthetic account passwords; browser Login Flow v2 and issued app passwords were not exercised.
- **LAB-006:** R34 returned default/forced/presentation/voiceroom/webinar for regular user and admin. R35 additionally returned channel/classified to the regular user and announcement to the admin. The full feature lists contained 104 and 108 entries respectively; the difference was announcement-preset, classified-conversations, preserve-conversation and promote-demote-owner. These are observations of these default fixtures, not version-number authorization rules.
- **LAB-007–009:** a regular user created a private group, invited another user, and both joined with actual room sessions. The member's returned role was ordinary user. A Unicode message was sent (201) and read by that member with matching message ID, body and author. A separately authenticated outsider received 404 for both the room and its chat history, and its room list omitted the private room.
- **LAB-010:** create public-results poll (201), ordinary member votes (200) with recorded option 0, member close denied (403), authoritative poll remains open, owner/author closes (200), member reads the closed poll with one voter, and a further vote is rejected (400). This tests one owner/author and one ordinary voter; separate non-moderator-author/moderator-only closure, native Close UI and race recovery remain pending.
- **LAB-011:** selected only because R35 returned the channel preset. The same owner successfully joined/left ordinary-group call **control state** (200/200), then created and joined a channel conversation whose channel attribute was set. Its call join was rejected (403), despite owner authority and working ordinary-room control. The requests used `flags=1` and no WebRTC peer or devices. R34's channel assertion was explicitly not applicable because the preset was absent.

The assertion counts include HTTP/envelope checks and are not feature counts. CT-001/002/004–007/010 have only the stated backend subsets exercised; no entire CT scenario is marked passed.

## Failed attempts and corrections

Earlier failures are retained, not relabelled as successful runs. Every attempt removed its two containers, two networks, two volumes and runtime directory.

| Evidence | Actual failure | Correction before successful run |
| --- | --- | --- |
| [Attempt 1](evidence/attempt-1.json) | Both images/packages verified; port-map handling raised `object of type 'NoneType' has no len()` before installed-server assertions. Docker had not published the requested mapping on the internal network. | Removed publication entirely; use the VM's direct internal-bridge access with exact trusted Host `127.0.0.1`. Assert no published ports and exactly one internal network. |
| [Attempt 2](evidence/attempt-2.json) | R34 installed the correct versions, then the integrity-output parser raised `Expecting value: line 1 column 1 (char 0)`. R35 installation failed with `The "--admin-pass" option requires a value.` | Handle the checker's empty successful output and explicitly require its signature file. The R35 secret was privately confirmed to begin with `-`; prepend a fixed non-option prefix while retaining all random entropy. Readiness now detects exited containers early. |
| [Attempt 3](evidence/attempt-3.json) | Both pairs passed identity/auth/presets/room/chat/isolation, then `LAB-010.member-votes.http` failed: actual HTTP/OCS 200, expected 201. | Match released-controller behavior with strict 200, then assert actual vote and closure semantics. Added the ordinary-room call-control comparison before the channel denial. |

The integrity checker only emits success text in verbose mode and may return zero when a signature is absent; requiring the signature file prevents treating a skipped check as verification. [Pinned server implementation](https://github.com/nextcloud/server/blob/ba900af933e61ffa005315b09d6454bfbbcbcc81/core/Command/Integrity/CheckApp.php)

Both released poll controllers return 200 for vote/read, while `docs/poll.md` lists 201. This discrepancy is documented here without changing the frozen specifications. [Talk24 controller](https://github.com/nextcloud/spreed/blob/0d736588dcfb42becf0e5688e0847e35d5b5efa2/lib/Controller/PollController.php), [Talk25 controller](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/lib/Controller/PollController.php), [Talk25 API prose](https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/docs/poll.md).

## Isolation, cleanup and limits

Each final container had one internal Docker bridge, **zero published host ports**, no dev-box forwards, a separate disposable data volume, a read-only administrator secret-file mount, and CPU/memory/process limits. Requests used HTTP on the private bridge; this is explicitly narrower than production TLS/auth acceptance and differs from Q.LAB-02's forwarded-loopback example. No public service, production data, firewall exception, TURN relay or Docker-socket mount was created. Runtime files were under restricted VM tmpfs directories; server passwords reached commands through secret-file/stdin mechanisms, not submitted command arguments.

Cleanup removed all run resources and runtime secrets before hibernation. The [post-cleanup inspection](evidence/post-cleanup.json) independently records zero lab-labelled containers/networks/volumes and zero lab runtime directories across all attempts. Docker reported AppArmor, default seccomp and cgroup namespace protections. Existing firewall default policies were inspected, not changed: IPv4 INPUT ACCEPT/FORWARD DROP and IPv6 INPUT/FORWARD ACCEPT. These defaults are not claimed as a lab firewall allowlist. Internal-network/no-publication configuration was checked; an external-host reachability probe was not run.

[Lifecycle evidence](evidence/lifecycle.json) records final VM state and estimated runtime/cost after cleanup. Public image caches and sanitized reports may remain in the hibernated environment; disposable accounts, databases, container writable layers and secret files were removed. This is logical resource deletion, not a forensic disk-erasure claim. No lab data was intentionally retained in a VM snapshot.

**Not demonstrated:** browser login/2FA/SSO, app-password lifecycle, revoked-account callback races, Keychain and safe fallback, native GPUI rendering or accessibility, direct-call/prejoin behavior, OS capture, actual WebRTC/media exchange, E2EE, HPB/TURN, recording, guest/file/federation workflows, full preset restrictions, recovery races, native performance or parity across either `.x` family. There was no native executable in this run. These results neither approve implementation gates nor complete a product feature.
