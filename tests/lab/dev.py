#!/usr/bin/env python3
"""Persistent, disposable CORE development lab. No native client or media qualification."""

import argparse
import base64
import contextlib
import fcntl
import http.cookiejar
import json
import os
from pathlib import Path
import platform
import re
import signal
import stat
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

sys.dont_write_bytecode = True
from lab import API, HERE, LABEL, USERS, Lab, LabError, LabInterrupted, NoRedirect, cleanup, digest, now, save


def client_url(value):
    """An explicit exception for this fixture only; never general HTTP support."""
    try:
        parsed = urllib.parse.urlsplit(value)
        port = parsed.port
    except ValueError as error:
        raise argparse.ArgumentTypeError("Invalid loopback URL") from error
    if (parsed.scheme != "http" or parsed.hostname != "127.0.0.1" or not port
            or not 1024 <= port <= 65535 or parsed.username or parsed.password
            or parsed.path not in ("", "/") or parsed.query or parsed.fragment
            or parsed.netloc != f"127.0.0.1:{port}"):
        raise argparse.ArgumentTypeError("Use exactly http://127.0.0.1:PORT at the server root (port 1024..65535)")
    return value.rstrip("/")


def private_json(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o600:
        raise LabError("Runtime files must be regular, owned by the current user, and mode 0600")
    if info.st_size > 1024 * 1024:
        raise LabError("Runtime record exceeds size bound")
    return json.loads(path.read_text())


@contextlib.contextmanager
def owned_state(value):
    path = Path(value).absolute()
    runtime = path.parent
    if (path.name != "state.json" or runtime.parent != Path("/run")
            or not re.fullmatch(r"nmtlab-[0-9]{8}t[0-9]{6}z-[0-9a-f]{8}-.+", runtime.name)):
        raise LabError("Expected the exact /run/nmtlab-.../state.json printed by up")
    info = runtime.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise LabError("Runtime directory must be owned by the current user, not a symlink, and mode 0700")
    private_json(path)
    with path.open("r") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise LabError("Another operation owns this lab; retry after it finishes") from error
        state = private_json(path)
        if state.get("runtime") != str(runtime) or not runtime.name.startswith(state["run_id"] + "-"):
            raise LabError("Runtime ownership mismatch")
        config = state.get("development", {})
        key = config.get("profile")
        if key not in ("r34", "r35") or not re.fullmatch(r"nmtlab-[0-9]{8}t[0-9]{6}z-[0-9a-f]{8}", state["run_id"]):
            raise LabError("Invalid development profile/run identity")
        prefix = state["run_id"] + "-" + key
        for kind, expected in (("containers", prefix), ("networks", prefix + "-net"), ("volumes", prefix + "-data")):
            if state[kind] not in ([], [expected]):
                raise LabError("Unexpected resource identity; refusing operation")
        client_url(config["client_url"])
        yield path, state


def clients():
    return {user: urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect(),
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())) for user in (*USERS, None)}


def restore(state):
    """Restore only private transport/fixture state, without provisioning anything."""
    lab = Lab.__new__(Lab)
    lab.pins = json.loads((HERE / "pins.json").read_text())
    lab.state, lab.run_id, lab.runtime = state, state["run_id"], Path(state["runtime"])
    config = state["development"]
    if config["pins_sha256"] != digest(HERE / "pins.json"):
        raise LabError("Pins changed; stop the old lab and provision a fresh profile")
    lab.base = config["vm_url"]
    lab.http_host = urllib.parse.urlsplit(config["client_url"]).netloc
    lab.output = Path(config["output"])
    lab.deadline = time.monotonic() + 180
    lab.container = state["containers"][0]
    credentials = private_json(lab.runtime / "credentials.json")
    lab.passwords = credentials["users"]
    if set(lab.passwords) != set(USERS):
        raise LabError("Unexpected synthetic account set")
    lab.sensitive = list(lab.passwords.values())
    lab.sensitive.extend(base64.b64encode(f"{u}:{p}".encode()).decode() for u, p in lab.passwords.items())
    lab.clients = clients()
    lab.current = {"assertions": [], "operations": []}
    lab.report = {"schema": 1, "scope": "development CORE, HTTP loopback only, no native/media qualification",
                  "run_id": lab.run_id, "started": now(), "profile": config["profile"],
                  "source_sha256": source_hashes(), "provision_source_sha256": config["source_sha256"],
                  "checks": lab.current}
    return lab


def source_hashes():
    return {"tests/lab/" + p.name: digest(p) for p in sorted(HERE.iterdir()) if p.is_file()}


def check_ownership(lab):
    for kind in ("container", "network", "volume"):
        name = lab.state[kind + "s"][0]
        field = ".Config.Labels" if kind == "container" else ".Labels"
        value = lab.docker(kind, "inspect", name, "--format", '{{index ' + field + ' "' + LABEL + '"}}')
        lab.require("DEV-ownership." + kind, value == lab.run_id)
    ports = json.loads(lab.docker("inspect", lab.container, "--format", '{{json .NetworkSettings.Ports}}'))
    lab.require("DEV-no-published-ports", not any((ports or {}).values()))
    network = lab.state["networks"][0]
    lab.require("DEV-internal-network", lab.docker("network", "inspect", network, "--format", "{{.Internal}}") == "true")
    networks = json.loads(lab.docker("inspect", lab.container, "--format", '{{json .NetworkSettings.Networks}}'))
    lab.require("DEV-single-network", list(networks) == [network])
    lab.require("DEV-owned-bridge-address", lab.base == "http://" + networks[network]["IPAddress"])
    lab.require("DEV-running", lab.docker("inspect", lab.container, "--format", "{{.State.Running}}") == "true")


def health(lab):
    check_ownership(lab)
    key = lab.state["development"]["profile"]
    pin = lab.pins["profiles"][key]
    status = lab.request(None, "GET", "/status.php", name="DEV-health")
    lab.require("DEV-ready", status.get("installed") and status.get("maintenance") is False
                and status.get("needsDbUpgrade") is False and status.get("versionstring") == pin["nextcloud_version"])
    talk = lab.occ("config:app:get", "spreed", "installed_version")
    lab.require("DEV-talk-version", talk == pin["talk_version"])
    caps = lab.request("lab_alice", "GET", "/ocs/v2.php/cloud/capabilities", name="DEV-capabilities")
    features = sorted(caps["capabilities"]["spreed"]["features"])
    lab.require("DEV-capabilities-present", {"chat-v2", "conversation-presets", "talk-polls"}.issubset(features))
    return {"ready": True, "profile": key, "nextcloud": status["versionstring"], "talk": talk,
            "capability_features": features, "run_id": lab.run_id}


def chat_check(lab):
    fixtures = private_json(lab.runtime / "fixtures.json")
    token = fixtures["group_token"]
    lab.sensitive.append(token)
    room = API + "v4/room/" + token
    for user in ("lab_alice", "lab_bob"):
        data = lab.request(user, "POST", room + "/participants/active", body={}, name="DEV-join." + user)
        lab.sensitive.append(data.get("sessionId", ""))
    msg = "Development check — Grüße aus dem Testlabor."
    sent = lab.request("lab_alice", "POST", API + "v1/chat/" + token, body={"message": msg},
                       expected=(201,), name="DEV-send")
    messages = lab.request("lab_bob", "GET", API + "v1/chat/" + token,
                           query={"lookIntoFuture": 0, "limit": 50, "setReadMarker": 0}, name="DEV-member-read")
    lab.require("DEV-member-same-message", any(m.get("id") == sent["id"] and m.get("message") == msg
                and m.get("actorId") == "lab_alice" for m in messages))
    lab.request("lab_outsider", "GET", room, expected=(404,), name="DEV-outsider-denied")
    for user in ("lab_alice", "lab_bob"):
        lab.request(user, "DELETE", room + "/participants/active", name="DEV-leave." + user)


def flow_request(lab, path, form=None):
    data = urllib.parse.urlencode(form or {}).encode()
    request = urllib.request.Request(lab.base + path, data=data, method="POST",
                                    headers={"Host": lab.http_host, "Accept": "application/json",
                                             "User-Agent": "NMT disposable development lab",
                                             "Content-Type": "application/x-www-form-urlencoded"})
    try:
        response = lab.clients[None].open(request, timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.code, response.read(1024 * 1024)


def flow_path(lab, url):
    parsed = urllib.parse.urlsplit(url)
    expected = urllib.parse.urlsplit(lab.state["development"]["client_url"])
    if (parsed.scheme, parsed.netloc) != (expected.scheme, expected.netloc) or parsed.query or parsed.fragment:
        raise LabError("Login Flow returned a different origin; no token sent")
    return parsed.path


def start_login(lab):
    code, raw = flow_request(lab, "/index.php/login/v2")
    lab.require("DEV-login-init", code == 200, {"http": code})
    flow = json.loads(raw)
    lab.sensitive.extend([flow["poll"]["token"], flow["login"]])
    flow_path(lab, flow["login"])
    path = flow_path(lab, flow["poll"]["endpoint"])
    save(lab.runtime / "login-flow.json", flow)
    code, _ = flow_request(lab, path, {"token": flow["poll"]["token"]})
    lab.require("DEV-login-pending", code == 404, {"http": code})
    return {"stage": "initiated; pending poll 404; browser authorization and app-password issuance NOT exercised",
            "private_flow_file": str(lab.runtime / "login-flow.json")}


def poll_login(lab):
    flow = private_json(lab.runtime / "login-flow.json")
    lab.sensitive.extend([flow["poll"]["token"], flow["login"]])
    path = flow_path(lab, flow["poll"]["endpoint"])
    code, raw = flow_request(lab, path, {"token": flow["poll"]["token"]})
    if code == 404:
        return {"stage": "pending or expired; no app password received", "http": 404}
    lab.require("DEV-login-poll", code == 200, {"http": code})
    account = json.loads(raw)
    lab.sensitive.append(account["appPassword"])
    lab.require("DEV-login-result-origin", account["server"].rstrip("/") == lab.state["development"]["client_url"])
    lab.require("DEV-login-synthetic-user", account["loginName"] in USERS)
    save(lab.runtime / "app-password.json", account)
    return {"stage": "app password received; native storage NOT exercised",
            "private_app_credential_file": str(lab.runtime / "app-password.json")}


def up(args):
    if not args.allow_loopback_http:
        raise LabError("Development HTTP requires the explicit --allow-loopback-http flag")
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "amd64"):
        raise LabError("This fixture requires Linux amd64")
    if not 30 <= args.budget_seconds <= 3600:
        raise LabError("Budget must be 30..3600 seconds")
    lab = Lab(args)
    lab.report["source_commit"] = "author working tree; integrator must bind recorded hashes to a committed candidate"
    lab.state["development"] = {"profile": args.profile, "client_url": args.client_url,
                                "output": str(lab.output), "pins_sha256": digest(HERE / "pins.json"),
                                "source_sha256": source_hashes(), "ready": False}
    save(lab.runtime / "state.json", lab.state)
    lock = (lab.runtime / "state.json").open("r")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    print(json.dumps({"recovery_state": str(lab.runtime / "state.json"), "run_id": lab.run_id}), flush=True)
    try:
        if lab.command("findmnt", "--noheadings", "--output", "FSTYPE", "--target", "/run").stdout.strip() != "tmpfs":
            raise LabError("/run must be tmpfs; do not snapshot runtime secrets")
        version = lab.docker("version", "--format", "{{.Server.Version}}")
        if int(version.split(".")[0]) < 28:
            raise LabError("Docker >=28 required")
        lab.report["runtime"] = {"os": platform.system(), "kernel": platform.release(),
                                  "python": platform.python_version(), "docker_server": version}
        lab.setup_profile(args.profile, lab.pins["profiles"][args.profile])
        save(lab.runtime / "credentials.json", {"schema": 1, "kind": "synthetic browser/server passwords; NOT app credentials",
                                               "users": lab.passwords})
        authority = urllib.parse.urlsplit(args.client_url).netloc
        # No path proxy, trusted proxy, wildcard domain, certificate or TLS override.
        lab.occ("config:system:delete", "trusted_domains")
        lab.occ("config:system:set", "trusted_domains", "0", "--value=" + authority)
        # The SSH transport preserves the actual Host and root path. Do not set
        # overwritehost: Nextcloud's trusted-domain check uses the effective host.
        lab.occ("config:system:set", "overwrite.cli.url", "--value=" + args.client_url)
        lab.http_host = authority
        lab.state["development"]["vm_url"] = lab.base
        lab.smoke(args.profile, lab.pins["profiles"][args.profile])
        rooms = lab.request("lab_alice", "GET", API + "v4/room", name="DEV-seeded-rooms")
        group = next(r for r in rooms if r.get("name") == "Fixture group")
        save(lab.runtime / "fixtures.json", {"group_token": group["token"], "room_count": len(rooms)})
        chat_check(lab)
        summary = health(lab)
        lab.state["development"]["ready"] = True
        save(lab.runtime / "state.json", lab.state)
        manifest = {**summary, "client_url": args.client_url, "vm_url": lab.base, "subpath": "/",
                    "state_file": str(lab.runtime / "state.json"),
                    "credentials_file": str(lab.runtime / "credentials.json"),
                    "fixtures_file": str(lab.runtime / "fixtures.json"),
                    "seed": {"users": list(USERS), "group": "Fixture group", "owner": "lab_alice",
                             "member": "lab_bob", "outsider": "lab_outsider", "closed_poll": True,
                             "channel": "channel" in lab.current["presets"]["lab_alice"]},
                    "transport": "HTTP on isolated bridge via SSH to VM; workstation loopback only; no TLS qualification",
                    "source_sha256": source_hashes()}
        save(lab.runtime / "manifest.json", manifest)
        lab.report.update(result="pass", scope="development CORE; persistent until explicit stop; no native/media qualification",
                          cleanup={"result": "not requested; development lab running"}, finished=now())
        save(lab.output / "provision.json", lab.report)
        print(json.dumps({"result": "ready", "manifest_file": str(lab.runtime / "manifest.json"),
                          "state_file": str(lab.runtime / "state.json"), "credentials_file": manifest["credentials_file"],
                          "client_url": args.client_url, "profile": args.profile}), flush=True)
        return 0
    except Exception as error:
        # An incomplete up has no usable environment; preserve failure evidence and clean it.
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        lab.report.update(result="fail", error=lab.redact(str(error)), finished=now())
        try:
            lab.report["cleanup"] = cleanup(lab.runtime / "state.json")
        except Exception as cleanup_error:
            lab.report["cleanup"] = {"result": "fail", "error": lab.redact(str(cleanup_error))}
        save(lab.output / "provision.json", lab.report)
        print(json.dumps({"result": "fail", "error": lab.report["error"], "cleanup": lab.report["cleanup"]}), flush=True)
        return 1
    finally:
        lock.close()


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    start = sub.add_parser("up", help="Provision and leave one fresh backend running")
    start.add_argument("--profile", choices=("r34", "r35"), required=True)
    start.add_argument("--client-url", type=client_url, required=True, help="Actual client-side forwarded loopback URL")
    start.add_argument("--allow-loopback-http", action="store_true")
    start.add_argument("--output", required=True, help="New evidence directory outside the source tree")
    start.add_argument("--budget-seconds", type=int, default=900)
    for action in ("status", "check", "stop", "reset", "login-start", "login-poll"):
        command = sub.add_parser(action)
        command.add_argument("--state", required=True)
        if action == "reset":
            command.add_argument("--profile", choices=("r34", "r35"), help="Optional fresh family; never reuse the old DB")
            command.add_argument("--output", required=True, help="New, empty evidence directory")
    args = parser.parse_args()

    def interrupted(signum, frame):
        raise LabInterrupted("Interrupted by signal " + str(signum))

    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        if args.action == "up":
            return up(args)
        reset_args = None
        with owned_state(args.state) as (path, state):
            config = state["development"]
            if args.action in ("stop", "reset"):
                # Removing the whole disposable DB invalidates all its credentials and sessions.
                result = cleanup(path)
                print(json.dumps(result), flush=True)
                if result["result"] != "pass":
                    return 1
                if args.action == "stop":
                    return 0
                reset_args = argparse.Namespace(profile=args.profile or config["profile"],
                    client_url=config["client_url"], allow_loopback_http=True, output=args.output, budget_seconds=900)
            else:
                if not config.get("ready"):
                    raise LabError("Provisioning incomplete; use stop with this state before retrying")
                lab = restore(state)
                summary = health(lab)
                if args.action == "check":
                    chat_check(lab)
                elif args.action == "login-start":
                    summary.update(start_login(lab))
                elif args.action == "login-poll":
                    summary.update(poll_login(lab))
                lab.report.update(result="pass", finished=now())
                evidence = lab.output / (args.action + "-" + str(time.time_ns()) + ".json")
                save(evidence, lab.report)
                print(json.dumps({**summary, "result": "pass", "evidence_file": str(evidence),
                                  "client_url": config["client_url"], "state_file": str(path)}), flush=True)
        return up(reset_args) if reset_args else 0
    except Exception as error:
        print(json.dumps({"result": "fail", "error": str(error) if isinstance(error, (LabError, argparse.ArgumentTypeError))
                          else type(error).__name__ + "; details withheld; inspect private state"}), flush=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
