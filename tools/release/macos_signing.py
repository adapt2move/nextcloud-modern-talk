"""Persistent signing for alpha and Developer ID distribution.

An internal alpha identity is pinned to its certificate and bundle identifier.
It changes neither end-user trust settings nor keychain item ACLs. Developer ID
keeps Apple's team requirement, hardened runtime and secure timestamp.
"""
import os
from pathlib import Path
import re

IDENTIFIER = "org.adapt2move.nextcloud-modern-talk"


def configuration(environ=None):
    env = os.environ if environ is None else environ
    identity = env.get("NMT_MACOS_SIGNING_IDENTITY", "")
    team = env.get("NMT_MACOS_SIGNING_TEAM", "")
    keychain = env.get("NMT_MACOS_SIGNING_KEYCHAIN", "")
    mode = env.get("NMT_MACOS_SIGNING_MODE", "developer-id")
    pin = env.get("NMT_MACOS_SIGNING_CERTIFICATE", "")
    required = env.get("NMT_MACOS_REQUIRE_SIGNED", "0")
    if required not in ("0", "1") or mode not in ("developer-id", "internal"):
        raise ValueError("Invalid macOS signing requirement/mode")
    if not identity and not team and not keychain and not pin:
        if required == "1":
            raise ValueError("Publishing requires a persistent macOS signing identity")
        return None
    if not re.fullmatch(r"[A-F0-9]{40}", identity):
        raise ValueError("Invalid or incomplete macOS signing identity")
    if mode == "developer-id":
        if not re.fullmatch(r"[A-Z0-9]{10}", team) or pin:
            raise ValueError("Invalid Developer ID signing team")
    elif team or pin != identity:
        raise ValueError("Internal signing requires the exact pinned certificate and no Apple team")
    if keychain and (not Path(keychain).is_absolute() or any(c in keychain for c in "\r\n\0")):
        raise ValueError("Invalid macOS signing keychain")
    return {"identity": identity, "team": team, "keychain": keychain, "mode": mode}


def description(config=None):
    config = configuration() if config is None else config
    if not config:
        return "ad-hoc; no Developer ID; not notarized"
    if config["mode"] == "internal":
        return "persistent internal certificate " + config["identity"] + "; no Developer ID; not notarized"
    return "Developer ID; team " + config["team"] + "; not notarized"


def requirement(config, identifier=None):
    if config["mode"] == "internal":
        result = 'certificate leaf = H"' + config["identity"] + '"'
    else:
        result = ('anchor apple generic and certificate leaf[subject.OU] = "'
                  + config["team"] + '" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists')
    if identifier:
        if identifier != IDENTIFIER:
            raise ValueError("Unexpected application signing identifier")
        result += ' and identifier "' + identifier + '"'
    return result


def sign(path, run, *, application=False):
    config = configuration()
    if not config:
        run(["codesign", "--force", "--sign", "-", "--timestamp=none", path])
        return
    args = ["codesign", "--force"]
    if config["keychain"]:
        args += ["--keychain", config["keychain"]]
    args += ["--sign", config["identity"]]
    if config["mode"] == "developer-id":
        args += ["--options", "runtime", "--timestamp"]
    else:
        # Preserve the alpha runtime profile. A self-signed certificate cannot
        # supply the Apple Team ID required by hardened library validation.
        args += ["--timestamp=none"]
    if application:
        args += ["--identifier", IDENTIFIER, "--entitlements", Path(__file__).with_name("macos-entitlements.plist")]
        if config["mode"] == "internal":
            args += ["--requirements", "=designated => " + requirement(config, IDENTIFIER)]
    run([*args, path])
    run(["codesign", "--verify", "--strict", "-R=" + requirement(config, IDENTIFIER if application else None), path])


def verify_application(path, run):
    config = configuration()
    if config:
        run(["codesign", "--verify", "--deep", "--strict", "-R=" + requirement(config, IDENTIFIER), path])
