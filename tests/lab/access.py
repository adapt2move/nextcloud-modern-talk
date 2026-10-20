#!/usr/bin/env python3
"""Workstation-only SSH/SFTP access to an existing lab; never builds or runs the app.

Uses the assigned agent-dev-box session's identity and verified host-key file.
No credentials pass through agent-dev-box job output or command arguments.
"""

import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import urllib.parse

FILES = ("manifest.json", "credentials.json", "fixtures.json", "login-flow.json", "app-password.json")


def command(args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=30)
    if result.returncode:
        # stderr can contain paths/host identities, but never expose file contents.
        raise RuntimeError(args[0] + " failed; check session, verified host key, port availability and private directory")
    return result.stdout


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")
    path.chmod(0o600)


def directory(value, create=False):
    path = Path(value).absolute()
    if create:
        path.mkdir(mode=0o700, parents=False, exist_ok=False)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o700 or info.st_uid != os.geteuid():
        raise RuntimeError("Private directory must be mode 0700, owned by this user, and not a symlink")
    path = path.resolve()  # macOS /tmp is a system symlink to /private/tmp.
    if len(str(path / "ssh.sock").encode()) > 95:
        raise RuntimeError("Use a short directory outside the repository, e.g. /tmp/nmt-access-UNIQUE")
    repo = Path(__file__).resolve().parents[2]
    if path == repo or repo in path.parents:
        raise RuntimeError("Access files must be outside the repository")
    return path


def read(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o600 or info.st_uid != os.geteuid():
        raise RuntimeError("Private files must be mode 0600, owned by this user, and not symlinks")
    return json.loads(path.read_text())


def session_transport(session):
    provider, identifier = session.split(":", 1)
    sessions = json.loads(command(["agent-dev-box", "status"]))["sessions"].values()
    selected = [s for s in sessions if s.get("provider") == provider and s.get("id") == identifier]
    if len(selected) != 1 or selected[0]["phase"] != "running":
        raise RuntimeError("The exact assigned session must be running")
    selected = selected[0]
    address = str(ipaddress.ip_address(selected["address"]))
    config = json.loads(command(["agent-dev-box", "config"]))
    identity = Path(config["ssh_identity"])
    known = identity.parent / ("hosts-" + str(selected["server_id"]))
    if not identity.is_file() or not known.is_file():
        raise RuntimeError("Configured SSH identity/verified host-key file missing; no trust bypass")
    return {"session": session, "address": address, "server_id": selected["server_id"],
            "ssh_options": ["-i", str(identity), "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=yes",
                            "-o", "UserKnownHostsFile=" + str(known), "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]}


def fetch(private, transport, filename):
    destination = private / filename
    if destination.exists() and (destination.is_symlink() or not destination.is_file()):
        raise RuntimeError("Refusing non-regular destination")
    address = transport["address"]
    remote = f"root@[{address}]" if ":" in address else "root@" + address
    command(["scp", "-q", *transport["ssh_options"], remote + ":" + transport["runtime"] + "/" + filename,
             str(destination)])
    destination.chmod(0o600)
    return destination


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    connect = sub.add_parser("connect")
    connect.add_argument("--session", required=True)
    connect.add_argument("--state", required=True, help="The exact remote /run/nmtlab-.../state.json")
    connect.add_argument("--directory", required=True, help="New, short private local directory outside the repository")
    for action in ("status", "fetch", "disconnect"):
        p = sub.add_parser(action)
        p.add_argument("--directory", required=True)
        if action == "fetch":
            p.add_argument("--file", choices=FILES, required=True)
    args = parser.parse_args()
    try:
        if args.action == "connect":
            if not re.fullmatch(r"/run/nmtlab-[0-9]{8}t[0-9]{6}z-[0-9a-f]{8}-[a-z0-9_]+/state.json", args.state):
                raise RuntimeError("Invalid remote state path")
            transport = session_transport(args.session)
            private = directory(args.directory, create=True)
            transport["runtime"] = str(Path(args.state).parent)
            write(private / "transport.json", transport)  # Recovery exists before starting a tunnel.
            manifest = read(fetch(private, transport, "manifest.json"))
            if manifest["state_file"] != args.state or manifest["profile"] not in ("r34", "r35"):
                raise RuntimeError("Manifest identity mismatch")
            client = urllib.parse.urlsplit(manifest["client_url"])
            backend = urllib.parse.urlsplit(manifest["vm_url"])
            if (client.scheme != "http" or client.netloc != f"127.0.0.1:{client.port}"
                    or not 1024 <= client.port <= 65535 or client.path or client.query or client.fragment):
                raise RuntimeError("Invalid client loopback origin")
            ip = ipaddress.IPv4Address(backend.hostname)
            if (not ip.is_private or ip.is_loopback or ip.is_link_local or backend.scheme != "http"
                    or backend.netloc != str(ip) or backend.path or backend.query or backend.fragment):
                raise RuntimeError("Expected the private, owned container bridge endpoint")
            control = str(private / "ssh.sock")
            command(["ssh", *transport["ssh_options"], "-M", "-S", control, "-f", "-N",
                     "-o", "ExitOnForwardFailure=yes", "-o", "GatewayPorts=no", "-o", "ServerAliveInterval=15",
                     "-o", "ServerAliveCountMax=3", "-L", f"127.0.0.1:{client.port}:{ip}:80",
                     "root@" + transport["address"]])
            print(json.dumps({"result": "connected", "client_url": manifest["client_url"],
                              "private_directory": str(private), "manifest_file": str(private / "manifest.json")}))
        else:
            private = directory(args.directory)
            transport = read(private / "transport.json")
            control = private / "ssh.sock"
            if args.action == "fetch":
                print(json.dumps({"result": "copied privately", "file": str(fetch(private, transport, args.file))}))
            elif args.action == "status":
                command(["ssh", *transport["ssh_options"], "-S", str(control), "-O", "check", "root@" + transport["address"]])
                print(json.dumps({"result": "tunnel running; backend readiness requires dev.py status"}))
            else:
                allowed = {*FILES, "transport.json", "ssh.sock"}
                if any(entry.name not in allowed for entry in private.iterdir()):
                    raise RuntimeError("Unexpected private entry; recovery state and tunnel retained")
                for name in (*FILES, "transport.json"):
                    path = private / name
                    if path.is_symlink() or (path.exists() and not path.is_file()):
                        raise RuntimeError("Unexpected private entry; recovery state and tunnel retained")
                if control.exists():
                    command(["ssh", *transport["ssh_options"], "-S", str(control), "-O", "exit", "root@" + transport["address"]])
                for name in (*FILES, "transport.json"):
                    path = private / name
                    if path.exists():
                        if path.is_symlink() or not path.is_file():
                            raise RuntimeError("Unexpected private entry; left untouched")
                        path.unlink()
                private.rmdir()  # Unexpected files/socket prevent a false cleanup success.
                print(json.dumps({"result": "disconnected", "private_files_removed": True}))
        return 0
    except Exception as error:
        print(json.dumps({"result": "fail", "error": str(error) if isinstance(error, RuntimeError) else type(error).__name__,
                          "recovery": "Use disconnect with the exact private directory if connect created one"}))
        return 1


if __name__ == "__main__":
    sys.exit(main())
