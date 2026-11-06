"""Stable Developer ID signing; ad-hoc is confined to non-publishing builds.

Never synthesize an identifier-only designated requirement. Apple's default
Developer ID requirement binds the application identifier to its issuing team.
The caller supplies an already unlocked, temporary CI keychain when needed.
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
    required = env.get("NMT_MACOS_REQUIRE_SIGNED", "0")
    if required not in ("0", "1"):
        raise ValueError("Invalid macOS signing requirement")
    if not identity and not team and not keychain:
        if required == "1":
            raise ValueError("Publishing requires a persistent Developer ID signing identity")
        return None
    if not re.fullmatch(r"[A-F0-9]{40}", identity) or not re.fullmatch(r"[A-Z0-9]{10}", team):
        raise ValueError("Invalid or incomplete macOS signing identity/team")
    if keychain and (not Path(keychain).is_absolute() or any(c in keychain for c in "\r\n\0")):
        raise ValueError("Invalid macOS signing keychain")
    return {"identity": identity, "team": team, "keychain": keychain}


def description(config=None):
    config = configuration() if config is None else config
    return ("Developer ID; team " + config["team"] + "; not notarized" if config
            else "ad-hoc; no Developer ID; not notarized")


def requirement(config, identifier=None):
    # The Developer ID Application leaf extension distinguishes distribution
    # identities from development/Mac App Store certificates of the same team.
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
    args = ["codesign", "--force", "--sign", config["identity"], "--options", "runtime", "--timestamp"]
    if config["keychain"]:
        args += ["--keychain", config["keychain"]]
    if application:
        args += ["--identifier", IDENTIFIER, "--entitlements", Path(__file__).with_name("macos-entitlements.plist")]
    run([*args, path])
    # An ad-hoc, self-signed, wrong-team or wrong-certificate-kind signature must
    # fail even if codesign itself reported success. Do not relax library validation.
    run(["codesign", "--verify", "--strict", "-R=" + requirement(config, IDENTIFIER if application else None), path])


def verify_application(path, run):
    config = configuration()
    if config:
        run(["codesign", "--verify", "--deep", "--strict", "-R=" + requirement(config, IDENTIFIER), path])
