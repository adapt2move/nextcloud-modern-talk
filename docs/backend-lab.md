# Disposable backend lab

The lab provisions pinned NC34.0.5/Talk24.0.6 (`r34`) or NC35.0.2/Talk25.0.6 (`r35`) with synthetic accounts and conversations. It does not establish native client support. See [lab requirements](quality/test-servers.md) and [harness details](../tests/lab/README.md).

Use Linux amd64, Docker 28+, Python 3.11+, tmpfs `/run`, about 4 GiB RAM and 10 GiB disk. Commands below run from the repository root in a disposable environment owned by the operator. In the managed agent workflow, use only the assigned dev-box session; no lab or VM operation is needed for static docs work.

## Provision and inspect

```sh
python3 tests/lab/dev.py up --profile r34 \
  --client-url http://127.0.0.1:8784 --allow-loopback-http \
  --output /absolute/new/private-output
python3 tests/lab/dev.py status --state /run/nmtlab-RUN/state.json
python3 tests/lab/dev.py check --state /run/nmtlab-RUN/state.json
```

Replace `RUN` with the exact state path printed by `up`. For `r35`, choose that profile and a separate origin/output directory. Run families sequentially on small environments. Never downgrade a database or reuse an R35 volume for R34. `up` remains running until explicit stop. An interrupted attempt may leave an owned runtime: inspect its recovery state before retrying.

Runtime directories are private (0700, files 0600), outside source/sync. Keep the manifest and credential files private. Primary synthetic server passwords are browser/API fixtures, not client app credentials or a product credential-store implementation. `check` exercises a bounded API subset, not UI/media health or full parity.

## Reach a remote lab

The server uses an unpublished internal Docker network. For a remote Docker host, use its verified SSH identity and host key:

```sh
python3 tests/lab/access.py connect --session "$LAB_SESSION" \
  --state /run/nmtlab-RUN/state.json --directory /absolute/new/private-access
python3 tests/lab/access.py status --directory /absolute/new/private-access
```

Use the exact printed `client_url`. The helper binds workstation IPv4 loopback and forwards directly to the owned container; it creates no public HTTP listener. A browser on another machine needs its own authenticated forward and matching explicit origin. Reset changes the container address, requiring disconnect/reconnect. A different client port requires fresh provisioning.

The explicit development-HTTP opt-in confines HTTP to the private bridge and workstation loopback, with SSH between machines. It grants no production TLS exception. Only the configured loopback authority is trusted; non-loopback origins, userinfo, subpaths, query strings and fragments are rejected. An HTTPS-only client cannot use this profile without an explicit development option.

## Messaging recovery checks

After building the `talk-adapters` tests with `backend-test`, the opt-in recovery
runner provisions both pinned server families, exercises typed messaging and
room controls, and injects gateway HTML 500/403 responses and dropped connections
into history reads:

```sh
python3 tests/lab/resilience.py --profile all --output /absolute/new/recovery-results
```

The same authenticated client must recover, send and retrieve every accepted
fixture message exactly once after each failure. The runner also verifies that
each fault was actually injected and removes its owned containers and private
credentials. This tests real server adapters and transport recovery; it does
not establish native UI, macOS input, production TLS or media behavior.

If needed, `access.py fetch --directory ... --file credentials.json` (or `fixtures.json`, `manifest.json`, `login-flow.json`, `app-password.json`) copies the named file privately without printing it. Never put secrets in command arguments, logs, screenshots or reports.

## Browser authorization preparation

`dev.py login-start --state ...` starts Login Flow v2 and saves the private flow. Read its login URL privately, authorize in a disposable browser, then use `dev.py login-poll --state ...`. HTTP 404 is pending or expired, not success; a completed poll writes private `app-password.json`. These helpers do not implement the native client's login/storage runtime or qualify SSO, revocation or races. Use `--help` for command options.

## Reset and cleanup

Close the lab-owned browser and disconnect the specific workstation tunnel before reset/stop:

```sh
python3 tests/lab/access.py disconnect --directory /absolute/new/private-access
python3 tests/lab/dev.py stop --state /run/nmtlab-RUN/state.json
```

`reset --state ... --output /absolute/new/private-output` removes the owned database and creates a new run; retain its new state path. Ownership mismatches must leave unrelated resources untouched. Never global-prune or delete other agents' inputs. Failed cleanup retains recovery state; report residual resources privately rather than claiming success.

Remove separately copied credentials and verify no owned containers, volumes, runtime files or access tunnels remain. In the managed agent workflow, retrieve sanitized results, inspect session stats and hibernate the assigned environment only after secret-bearing cleanup. Preserve recovery snapshots and unrelated sessions. Hibernation alone does not delete credentials or data.

HPB/TURN, native capture, real calls, production TLS, macOS storage/permissions, accessibility and performance need separate qualification. HTTP call-control success is not media exchange.
