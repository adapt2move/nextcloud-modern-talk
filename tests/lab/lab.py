#!/usr/bin/env python3
"""Disposable, pinned Nextcloud/Talk API fixtures. Python standard library only.

Run via agent-dev-box on a disposable Linux amd64 VM. This is a backend probe,
not a native client or a substitute for the product acceptance matrix.
"""

import argparse
import base64
import datetime
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import platform
import re
import secrets
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
LABEL = "org.nextcloud-modern-talk.lab-run"
API = "/ocs/v2.php/apps/spreed/api/"
USERS = ("lab_admin", "lab_alice", "lab_bob", "lab_outsider")


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def save(path, data):
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    path.chmod(0o600)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class LabError(Exception):
    pass


class LabInterrupted(LabError):
    pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # Do not forward test credentials to a redirect target.
        return None


class Lab:
    def __init__(self, args):
        self.pins = json.loads((HERE / "pins.json").read_text())
        self.deadline = time.monotonic() + args.budget_seconds
        self.run_id = "nmtlab-" + time.strftime("%Y%m%dt%H%M%Sz", time.gmtime()) + "-" + secrets.token_hex(4)
        self.output = Path(args.output).resolve()
        if self.output == REPO or REPO in self.output.parents:
            raise LabError("Evidence output must be outside the repository; review it before copying")
        self.output.mkdir(mode=0o700, parents=True, exist_ok=True)
        if any(self.output.iterdir()):
            raise LabError("Evidence output must be empty; never overwrite a prior run")
        # /run is tmpfs on the qualified lab VM. Never use the synced workspace.
        self.runtime = Path(tempfile.mkdtemp(prefix=self.run_id + "-", dir="/run"))
        self.runtime.chmod(0o700)
        self.state = {"schema": 1, "run_id": self.run_id, "runtime": str(self.runtime),
                      "containers": [], "networks": [], "volumes": []}
        save(self.runtime / "state.json", self.state)
        self.sensitive = []
        self.report = {"schema": 1, "run_id": self.run_id, "started": now(),
                       "scope": self.pins["profile_kind"], "result": "running",
                       "specification_commit": self.pins["specification_commit"],
                       "source_commit": "uncommitted bootstrap; bind these file hashes at review",
                       "source_sha256": {str(p.relative_to(REPO)): digest(p)
                                         for p in sorted(HERE.iterdir()) if p.is_file()},
                       "profiles": {}, "cleanup": {"result": "pending"}}
        self.current = None
        self.container = None
        self.base = None
        self.passwords = {}
        self.clients = {}
        # The development CLI can set its explicit forwarded origin after setup.
        self.http_host = "127.0.0.1"

    def redact(self, text):
        for value in sorted(self.sensitive, key=len, reverse=True):
            if value:
                text = text.replace(value, "[redacted]")
        return text.replace(str(self.runtime), "[runtime]").replace(str(REPO), "[repository]")

    def command(self, *args, stdin=None, timeout=120, check=True):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise LabError("Run time budget exhausted")
        try:
            result = subprocess.run(args, input=stdin, text=True, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, timeout=min(timeout, remaining))
        except subprocess.TimeoutExpired as error:
            raise LabError("Bounded command timeout: " + args[0]) from error
        if check and result.returncode:
            # Keep the actual diagnostic, but never echo credentials or private paths.
            raise LabError(f"{args[0]} exited {result.returncode}: " + self.redact(result.stdout[-6000:]))
        return result

    def docker(self, *args, **kwargs):
        return self.command("docker", *args, **kwargs).stdout.strip()

    def occ(self, *args, **kwargs):
        return self.docker("exec", "--user", "www-data", self.container, "php", "occ", *args, **kwargs)

    def remember(self, kind, name):
        self.state[kind].append(name)
        save(self.runtime / "state.json", self.state)

    def require(self, name, condition, observed=None):
        row = {"id": name, "result": "pass" if condition else "fail"}
        if observed is not None:
            row["observed"] = observed
        self.current["assertions"].append(row)
        if not condition:
            raise LabError("Assertion failed: " + name)

    def request(self, actor, method, path, *, body=None, query=None, expected=(200,), name=None):
        url = self.base + path
        if query:
            url += "?" + urllib.parse.urlencode(query)
        headers = {"Accept": "application/json", "OCS-APIRequest": "true", "Host": self.http_host}
        if actor is not None:
            credentials = base64.b64encode(f"{actor}:{self.passwords[actor]}".encode()).decode()
            headers["Authorization"] = "Basic " + credentials
        data = None if body is None else json.dumps(body).encode()
        if data is not None:
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            response = self.clients[actor].open(request, timeout=min(20, max(1, self.deadline - time.monotonic())))
        except urllib.error.HTTPError as error:
            response = error
        with response:
            raw = response.read(4 * 1024 * 1024)
            status = response.code
        try:
            payload = json.loads(raw)
        except ValueError as error:
            raise LabError(f"Non-JSON response for {name or method}; HTTP {status}; body withheld") from error
        ocs = payload.get("ocs")
        meta = ocs.get("meta", {}) if isinstance(ocs, dict) else {}
        # Use a logical operation label: never publish room/session tokens, cookies or headers.
        row = {"operation": name, "actor": actor or "anonymous", "http_status": status,
               "ocs_statuscode": meta.get("statuscode"), "expected_http": list(expected)}
        self.current["operations"].append(row)
        self.require(name + ".http", status in expected, {"http": status, "ocs": meta.get("statuscode")})
        if path.startswith("/ocs/"):
            self.require(name + ".ocs-envelope", isinstance(ocs, dict) and "data" in ocs)
            if status < 400:
                self.require(name + ".ocs-success", meta.get("status") == "ok")
        return ocs["data"] if ocs else payload

    def setup_profile(self, key, pin):
        self.current = {"result": "running", "started": now(), "pins": pin,
                        "operations": [], "assertions": []}
        self.report["profiles"][key] = self.current
        print(f"{key}: verifying pinned image and package", flush=True)
        self.docker("pull", "--platform", self.pins["platform"], pin["image"], timeout=480)
        identity = json.loads(self.docker("image", "inspect", pin["image"], "--format",
                                         '{"id":{{json .Id}},"digests":{{json .RepoDigests}},"os":{{json .Os}},"architecture":{{json .Architecture}}}'))
        self.current["image_observed"] = identity
        self.require("LAB-001.image-platform", identity["os"] == "linux" and identity["architecture"] == "amd64")
        expected_digest = pin["image"].split("@", 1)[1]
        self.require("LAB-001.image-digest", any(d.endswith("@" + expected_digest) for d in identity["digests"]))
        archive = self.runtime / f"{key}-spreed.tar.gz"
        sha = hashlib.sha256()
        length = 0
        with urllib.request.urlopen(pin["talk_url"], timeout=60) as response, archive.open("wb") as out:
            while block := response.read(1024 * 1024):
                length += len(block)
                if length > 350 * 1024 * 1024 or time.monotonic() > self.deadline:
                    raise LabError("Talk package download exceeded size/time bound")
                sha.update(block)
                out.write(block)
        self.current["package_observed"] = {"sha256": sha.hexdigest(), "bytes": length}
        self.require("LAB-001.package-sha256", sha.hexdigest() == pin["talk_sha256"])

        # The image passes the admin password as a separate occ option value.
        # A random leading '-' is parsed as an option; prefix without reducing entropy.
        self.passwords = {user: "Nmt_" + secrets.token_urlsafe(36) for user in USERS}
        self.sensitive.extend(self.passwords.values())
        self.sensitive.extend(base64.b64encode(f"{u}:{p}".encode()).decode() for u, p in self.passwords.items())
        secret = self.runtime / f"{key}-admin-password"
        secret.write_text(self.passwords["lab_admin"])
        secret.chmod(0o600)
        network = self.run_id + "-" + key + "-net"
        volume = self.run_id + "-" + key + "-data"
        self.container = self.run_id + "-" + key
        for kind, name in (("networks", network), ("volumes", volume), ("containers", self.container)):
            self.remember(kind, name)  # Record intended ownership before creation.
        self.docker("network", "create", "--internal", "--label", LABEL + "=" + self.run_id, network)
        self.docker("volume", "create", "--label", LABEL + "=" + self.run_id, volume)
        self.docker("run", "--detach", "--name", self.container, "--label", LABEL + "=" + self.run_id,
                    "--platform", self.pins["platform"], "--network", network,
                    "--memory", "2g", "--cpus", "2", "--pids-limit", "256",
                    "--security-opt", "no-new-privileges:true", "--log-opt", "max-size=1m", "--log-opt", "max-file=1",
                    "--mount", f"type=volume,src={volume},dst=/var/www/html",
                    "--mount", f"type=bind,src={secret},dst=/run/secrets/admin-password,readonly",
                    "--env", "NEXTCLOUD_ADMIN_USER=lab_admin", "--env", "NEXTCLOUD_ADMIN_PASSWORD_FILE=/run/secrets/admin-password",
                    "--env", "SQLITE_DATABASE=lab.sqlite3", "--env", "NEXTCLOUD_TRUSTED_DOMAINS=127.0.0.1",
                    pin["image"], timeout=90)
        port = json.loads(self.docker("inspect", self.container, "--format", '{{json .NetworkSettings.Ports}}'))
        self.require("LAB-002.no-published-ports", not any((port or {}).values()))
        internal = self.docker("network", "inspect", network, "--format", "{{.Internal}}")
        self.require("LAB-002.internal-network", internal == "true")
        self.current["network"] = {"host_binding": "none; VM reaches internal bridge directly", "published_ports": [],
                                   "container_network_internal": True, "tls": False,
                                   "forwarded_ports": [], "database": "SQLite in disposable volume"}
        networks = json.loads(self.docker("inspect", self.container, "--format", '{{json .NetworkSettings.Networks}}'))
        self.require("LAB-002.single-network", list(networks) == [network])
        self.base = "http://" + networks[network]["IPAddress"]
        self.clients = {user: urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect(),
                         urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())) for user in (*USERS, None)}
        installed = None
        end = min(self.deadline, time.monotonic() + 240)
        while time.monotonic() < end:
            try:
                status_request = urllib.request.Request(self.base + "/status.php", headers={"Host": "127.0.0.1"})
                with self.clients[None].open(status_request, timeout=4) as response:
                    installed = json.load(response)
                if installed.get("installed") and not installed.get("maintenance"):
                    break
            except (OSError, ValueError):
                pass
            if self.docker("inspect", self.container, "--format", "{{.State.Running}}") != "true":
                logs = self.docker("logs", "--tail", "30", self.container, check=False)
                raise LabError("Container exited before readiness: " + self.redact(logs))
            time.sleep(2)
        else:
            logs = self.docker("logs", "--tail", "30", self.container, check=False)
            raise LabError("Installation readiness timed out: " + self.redact(logs))
        self.current["status"] = {k: installed.get(k) for k in ("installed", "maintenance", "needsDbUpgrade", "version", "versionstring", "productname")}
        self.require("LAB-003.installed-version", installed.get("versionstring") == pin["nextcloud_version"]
                     and installed.get("needsDbUpgrade") is False)
        self.occ("config:system:set", "appstoreenabled", "--type=boolean", "--value=false")
        self.occ("config:system:set", "updatechecker", "--type=boolean", "--value=false")
        self.docker("cp", str(archive), self.container + ":/tmp/spreed.tar.gz")
        self.docker("exec", self.container, "sh", "-ec",
                    "tar -xzf /tmp/spreed.tar.gz -C /var/www/html/custom_apps; "
                    "chown -R www-data:www-data /var/www/html/custom_apps/spreed; rm /tmp/spreed.tar.gz")
        self.occ("app:enable", "spreed", timeout=120)
        app_version = self.occ("config:app:get", "spreed", "installed_version")
        self.current["talk_installed_version"] = app_version
        self.require("LAB-003.talk-installed-version", app_version == pin["talk_version"])
        self.docker("exec", self.container, "test", "-s", "/var/www/html/custom_apps/spreed/appinfo/signature.json")
        integrity = self.occ("integrity:check-app", "spreed", "--output=json")
        # These releases emit nothing on success even with --output=json. They
        # return zero on missing signatures too, so the explicit check above is mandatory.
        self.current["talk_integrity"] = {"signature_present": True, "exit_code": 0, "stdout": integrity}
        self.require("LAB-003.talk-integrity", integrity == "")
        manifest_code = ('$x=simplexml_load_file("/var/www/html/custom_apps/spreed/appinfo/info.xml");'
                         'echo json_encode(["version"=>(string)$x->version,'
                         '"nextcloud_min"=>(string)$x->dependencies->nextcloud["min-version"],'
                         '"nextcloud_max"=>(string)$x->dependencies->nextcloud["max-version"]]);')
        manifest = json.loads(self.docker("exec", self.container, "php", "-r", manifest_code))
        self.current["talk_app_manifest"] = manifest
        major = pin["nextcloud_version"].split(".")[0]
        self.require("LAB-003.app-manifest", manifest == {"version": pin["talk_version"],
                     "nextcloud_min": major, "nextcloud_max": major})
        version_code = 'echo json_encode(["php"=>PHP_VERSION,"sqlite"=>SQLite3::version()["versionString"]]);'
        self.current["backend_versions"] = json.loads(self.docker("exec", self.container, "php", "-r", version_code))
        for user in USERS[1:]:
            # The password crosses stdin only; it is not in docker inspect, argv or evidence.
            self.docker("exec", "--interactive", "--user", "www-data", self.container, "sh", "-ec",
                        'IFS= read -r OC_PASS; export OC_PASS; exec php occ user:add --password-from-env "$1"',
                        "sh", user, stdin=self.passwords[user] + "\n")
        print(f"{key}: installed exact versions; running API checks", flush=True)

    def smoke(self, key, pin):
        req = self.request
        for user in USERS:
            data = req(user, "GET", "/ocs/v2.php/cloud/user", name="LAB-004.authenticate." + user)
            self.require("LAB-004.identity." + user, data.get("id") == user)
        req(None, "GET", API + "v4/room", expected=(401,), name="LAB-004.unauthenticated-room-list")
        caps = req("lab_alice", "GET", "/ocs/v2.php/cloud/capabilities", name="LAB-005.capabilities")
        talk = caps["capabilities"]["spreed"]
        features = sorted(talk["features"])
        self.current["capability_features"] = features
        self.require("LAB-005.required-capabilities", {"conversation-presets", "chat-v2", "talk-polls"}.issubset(features))
        config = talk.get("config", {})
        self.current["config_selection"] = {section: {k: values[k] for k in allowed if k in values}
            for section, allowed in {"chat": ("max-length",), "call": ("enabled", "recording", "recording-consent"),
                                     "conversations": ("can-create",)}.items()
            if isinstance(values := config.get(section), dict)}
        for actor, expected in (("lab_alice", pin["regular_presets"]), ("lab_admin", pin["admin_presets"])):
            presets = req(actor, "GET", API + "v1/presets/room", name="LAB-006.presets." + actor)
            observed = sorted(p["identifier"] for p in presets)
            self.current.setdefault("presets", {})[actor] = observed
            self.require("LAB-006.baseline-presets." + actor, observed == expected, observed)

        room = req("lab_alice", "POST", API + "v4/room", body={"roomType": 2, "roomName": "Fixture group"},
                   expected=(201,), name="LAB-007.create-group")
        token = room["token"]
        self.sensitive.append(token)
        room_path = API + "v4/room/" + token
        req("lab_alice", "POST", room_path + "/participants", body={"newParticipant": "lab_bob", "source": "users"},
            name="LAB-007.invite-member")
        for user in ("lab_alice", "lab_bob"):
            joined = req(user, "POST", room_path + "/participants/active", body={}, name="LAB-007.join." + user)
            self.require("LAB-007.session." + user, bool(joined.get("sessionId")))
            self.sensitive.append(joined["sessionId"])
        member = req("lab_bob", "GET", room_path, name="LAB-007.verify-ordinary-member")
        self.require("LAB-007.member-role", member.get("participantType") == 3)
        msg = "Nur für diese Unterhaltung — Grüße aus dem Testlabor."
        message = req("lab_alice", "POST", API + "v1/chat/" + token, body={"message": msg},
                      expected=(201,), name="LAB-008.send-message")
        history = req("lab_bob", "GET", API + "v1/chat/" + token,
                      query={"lookIntoFuture": 0, "limit": 50, "setReadMarker": 0}, name="LAB-008.member-reads")
        self.require("LAB-008.same-message-and-author", any(m.get("id") == message["id"] and m.get("message") == msg
                     and m.get("actorId") == "lab_alice" for m in history))
        req("lab_outsider", "GET", room_path, expected=(404,), name="LAB-009.outsider-room-denied")
        req("lab_outsider", "GET", API + "v1/chat/" + token,
            query={"lookIntoFuture": 0, "limit": 10}, expected=(404,), name="LAB-009.outsider-chat-denied")
        outsiders = req("lab_outsider", "GET", API + "v4/room", name="LAB-009.outsider-list")
        self.require("LAB-009.no-room-leak", all(r.get("token") != token for r in outsiders))

        poll_path = API + "v1/poll/" + token
        poll = req("lab_alice", "POST", poll_path, body={"question": "Wann treffen wir uns?", "options": ["Morgen", "Später"],
                   "resultMode": 0, "maxVotes": 1}, expected=(201,), name="LAB-010.create-poll")
        item = poll_path + "/" + str(poll["id"])
        voted = req("lab_bob", "POST", item, body={"optionIds": [0]}, name="LAB-010.member-votes")
        self.require("LAB-010.vote-recorded", voted.get("votedSelf") == [0])
        req("lab_bob", "DELETE", item, expected=(403,), name="LAB-010.voter-cannot-close")
        # Released controllers return 200 for GET/vote despite docs/poll.md's 201.
        opened = req("lab_alice", "GET", item, name="LAB-010.still-open")
        self.require("LAB-010.denial-does-not-close", opened["status"] == 0)
        closed = req("lab_alice", "DELETE", item, name="LAB-010.author-closes")
        self.require("LAB-010.closed-status", closed["status"] == 1)
        fetched = req("lab_bob", "GET", item, name="LAB-010.closed-readable")
        self.require("LAB-010.closed-result-retained", fetched["status"] == 1 and fetched["numVoters"] == 1)
        req("lab_bob", "POST", item, body={"optionIds": [1]}, expected=(400,), name="LAB-010.closed-vote-denied")

        # Gate this extra invariant on the server's advertised preset, not its major version.
        if "channel" in self.current["presets"]["lab_alice"]:
            # Positive control: distinguish the channel invariant from globally disabled calls.
            req("lab_alice", "POST", API + "v4/call/" + token, body={"flags": 1, "silent": True},
                name="LAB-011.ordinary-room-control-join")
            req("lab_alice", "DELETE", API + "v4/call/" + token, name="LAB-011.ordinary-room-control-leave")
            channel = req("lab_alice", "POST", API + "v4/room",
                          body={"roomType": 2, "roomName": "Fixture channel", "preset": "channel"},
                          expected=(201,), name="LAB-011.create-channel")
            self.sensitive.append(channel["token"])
            self.require("LAB-011.channel-attribute", channel.get("attributes", 0) & 8 == 8)
            path = API + "v4/room/" + channel["token"]
            req("lab_alice", "POST", path + "/participants/active", body={}, name="LAB-011.owner-joins-channel")
            req("lab_alice", "POST", API + "v4/call/" + channel["token"],
                body={"flags": 1, "silent": True}, expected=(403,), name="LAB-011.channel-owner-call-denied")
        else:
            self.current["assertions"].append({"id": "LAB-011.channel-owner-call-denied", "result": "not applicable",
                                              "reason": "Channel preset absent in observed regular-user preset list"})
        self.current.update(result="pass", finished=now())

    def run(self, profiles):
        self.report["profiles"] = {key: {"result": "not run"} for key in profiles}
        self.report["runtime"] = {"os": platform.system(), "architecture": platform.machine(),
                                  "kernel": platform.release(), "python": platform.python_version()}
        if platform.system() != "Linux" or platform.machine() not in ("x86_64", "amd64"):
            raise LabError("This fixture is pinned to Linux amd64")
        version = self.docker("version", "--format", "{{.Server.Version}}")
        self.report["runtime"]["docker_server"] = version
        if int(version.split(".")[0]) < 28:
            raise LabError("Docker >=28 required for qualified localhost-publishing behavior")
        for key in profiles:
            try:
                self.setup_profile(key, self.pins["profiles"][key])
                self.smoke(key, self.pins["profiles"][key])
            except Exception as error:
                if self.current is None:
                    raise
                self.current.update(result="fail", error=self.redact(str(error)), finished=now())
                print(f"{key}: failed: {self.redact(str(error))}", flush=True)
                if isinstance(error, LabInterrupted):
                    raise
            finally:
                # Keep resource memory bounded; all volumes/networks are removed in final cleanup.
                if self.container:
                    self.docker("stop", "--time", "10", self.container, timeout=30, check=False)
                self.current = None
                self.container = None
        self.report["result"] = "pass" if all(p["result"] == "pass" for p in self.report["profiles"].values()) else "fail"


def cleanup(state_path):
    """Remove only recorded resources bearing this exact run label; fail closed."""
    state_path = Path(state_path).resolve()
    state = json.loads(state_path.read_text())
    run_id = state["run_id"]
    if not re.fullmatch(r"nmtlab-[0-9]{8}t[0-9]{6}z-[0-9a-f]{8}", run_id):
        raise LabError("Invalid cleanup run identity")
    runtime = Path(state["runtime"])
    if runtime != state_path.parent or runtime.parent != Path("/run") or not runtime.name.startswith(run_id + "-"):
        raise LabError("Refusing cleanup outside the recorded /run fixture directory")
    removed = {"containers": 0, "volumes": 0, "networks": 0}
    errors = []
    for kind, commands in (("containers", ("container", "rm", "--force")),
                           ("volumes", ("volume", "rm")), ("networks", ("network", "rm"))):
        object_kind = {"containers": "container", "volumes": "volume", "networks": "network"}[kind]
        for name in state[kind]:
            fmt = '{{index .Config.Labels "' + LABEL + '"}}' if kind == "containers" else '{{index .Labels "' + LABEL + '"}}'
            inspect = subprocess.run(["docker", object_kind, "inspect", name, "--format", fmt],
                                     capture_output=True, text=True, timeout=20)
            if inspect.returncode:
                if "No such" in inspect.stderr or "not found" in inspect.stderr:
                    continue
                errors.append(kind + ": cannot verify recorded resource")
                continue
            if inspect.stdout.strip() != run_id:
                errors.append(kind + ": ownership label mismatch; untouched")
                continue
            result = subprocess.run(["docker", *commands, name], capture_output=True, text=True, timeout=45)
            if result.returncode:
                errors.append(kind + ": remove failed")
            else:
                removed[kind] += 1
    remaining = {}
    for kind, listing in (("containers", ["ps", "--all", "--quiet"]),
                           ("volumes", ["volume", "ls", "--quiet"]),
                           ("networks", ["network", "ls", "--quiet"])):
        result = subprocess.run(["docker", *listing, "--filter", "label=" + LABEL + "=" + run_id],
                                capture_output=True, text=True, timeout=20)
        remaining[kind] = len(result.stdout.split()) if result.returncode == 0 else "unknown"
        if result.returncode:
            errors.append(kind + ": residual-resource query failed")
    clean = not errors and all(v == 0 for v in remaining.values())
    if clean:
        shutil.rmtree(runtime)
    return {"result": "pass" if clean else "fail", "removed": removed, "remaining": remaining,
            "runtime_secrets_removed": not runtime.exists(), "errors": errors, "finished": now()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    run = sub.add_parser("run")
    run.add_argument("--profile", choices=("all", "r34", "r35"), default="all")
    run.add_argument("--output", required=True, help="New evidence directory outside the repository")
    run.add_argument("--budget-seconds", type=int, default=1800)
    remove = sub.add_parser("cleanup")
    remove.add_argument("--state", required=True, help="Private /run/nmtlab-*/state.json recovery record")
    args = parser.parse_args()
    os.umask(0o077)
    if args.action == "cleanup":
        result = cleanup(args.state)
        print(json.dumps(result, indent=2))
        return 0 if result["result"] == "pass" else 1
    if not 30 <= args.budget_seconds <= 3600:
        parser.error("--budget-seconds must be between 30 and 3600")
    lab = Lab(args)
    print("run: " + lab.run_id, flush=True)
    print("recovery state: " + str(lab.runtime / "state.json"), flush=True)

    def interrupted(signum, frame):
        raise LabInterrupted("Interrupted by signal " + str(signum))

    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        lab.run(list(lab.pins["profiles"]) if args.profile == "all" else [args.profile])
    except Exception as error:
        lab.report.update(result="fail", error=lab.redact(str(error)))
    finally:
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        try:
            lab.report["cleanup"] = cleanup(lab.runtime / "state.json")
        except Exception as error:
            lab.report["cleanup"] = {"result": "fail", "error": lab.redact(str(error))}
        if lab.report["cleanup"]["result"] != "pass":
            lab.report["result"] = "fail"
        lab.report["finished"] = now()
        save(lab.output / "report.json", lab.report)
        print(json.dumps({"result": lab.report["result"], "cleanup": lab.report["cleanup"]}, indent=2), flush=True)
    return 0 if lab.report["result"] == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
