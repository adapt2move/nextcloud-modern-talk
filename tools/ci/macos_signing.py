#!/usr/bin/env python3
"""Import the existing release identity into one disposable runner keychain.

No default keychain/search-list changes, credential output or generated release
identity. Missing credentials fail a publishing build before compilation.
"""
import argparse
import base64
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import tempfile


def run(arguments):
    result = subprocess.run(arguments, capture_output=True, text=True, timeout=30)
    if result.returncode:
        # security's stderr may include certificate/keychain/provider details.
        raise ValueError("Temporary macOS signing keychain operation failed")
    return result.stdout


def credentials(env):
    encoded = env.get("MACOS_CERTIFICATE_P12_BASE64", "")
    password = env.get("MACOS_CERTIFICATE_PASSWORD", "")
    team = env.get("MACOS_SIGNING_TEAM", "")
    if not encoded or not password or not re.fullmatch(r"[A-Z0-9]{10}", team):
        raise ValueError("Publishing needs MACOS_CERTIFICATE_P12_BASE64, MACOS_CERTIFICATE_PASSWORD and MACOS_SIGNING_TEAM")
    if len(encoded) > 350_000:
        raise ValueError("macOS signing certificate exceeds its size limit")
    encoded = "".join(encoded.split())
    try:
        data = base64.b64decode(encoded, validate=True)
    except ValueError:
        raise ValueError("Invalid macOS signing certificate encoding") from None
    if not data or len(data) > 256 * 1024:
        raise ValueError("Invalid macOS signing certificate size")
    return data, password, team


def cleanup(directory):
    root = Path(os.environ["RUNNER_TEMP"]).resolve()
    directory = Path(directory)
    if (directory.is_symlink() or not directory.resolve().is_relative_to(root)
            or directory.parent.resolve() != root or not directory.name.startswith("nmt-signing-")):
        raise ValueError("Invalid temporary signing directory")
    keychain = directory / "release.keychain-db"
    if keychain.exists():
        run(["security", "delete-keychain", str(keychain)])
    shutil.rmtree(directory)


def prepare():
    data, password, team = credentials(os.environ)
    directory = Path(tempfile.mkdtemp(prefix="nmt-signing-", dir=os.environ["RUNNER_TEMP"]))
    directory.chmod(0o700)
    keychain = directory / "release.keychain-db"
    certificate = directory / "identity.p12"
    try:
        with certificate.open("xb") as output:
            certificate.chmod(0o600)
            output.write(data)
        # This password protects only the temporary local keychain. It is not a
        # replacement release credential and never enters logs or GITHUB_ENV.
        local_password = secrets.token_hex(32)
        run(["security", "create-keychain", "-p", local_password, str(keychain)])
        run(["security", "set-keychain-settings", "-lut", "7200", str(keychain)])
        run(["security", "unlock-keychain", "-p", local_password, str(keychain)])
        run(["security", "import", str(certificate), "-k", str(keychain), "-P", password, "-T", "/usr/bin/codesign"])
        run(["security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", local_password, str(keychain)])
        identities = run(["security", "find-identity", "-v", "-p", "codesigning", str(keychain)])
        fingerprints = re.findall(r'\b([A-F0-9]{40}) "Developer ID Application: [^"\r\n]+ \(' + re.escape(team) + r'\)"', identities)
        if len(fingerprints) != 1:
            raise ValueError("Exactly one valid Developer ID Application identity of the configured team is required")
        with Path(os.environ["GITHUB_ENV"]).open("a") as output:
            output.write("NMT_MACOS_REQUIRE_SIGNED=1\nNMT_MACOS_SIGNING_IDENTITY=" + fingerprints[0]
                         + "\nNMT_MACOS_SIGNING_TEAM=" + team + "\nNMT_MACOS_SIGNING_KEYCHAIN=" + str(keychain)
                         + "\nNMT_MACOS_SIGNING_TEMP=" + str(directory) + "\n")
    except BaseException:
        cleanup(directory)
        raise
    finally:
        certificate.unlink(missing_ok=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cleanup", action="store_true")
    arguments = parser.parse_args()
    try:
        if arguments.cleanup:
            if directory := os.environ.get("NMT_MACOS_SIGNING_TEMP"):
                cleanup(directory)
        elif os.environ.get("NMT_MACOS_REQUIRE_SIGNED", "1") == "1":
            prepare()
        elif os.environ.get("NMT_MACOS_REQUIRE_SIGNED") != "0":
            raise ValueError("Invalid macOS signing requirement")
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        raise SystemExit(str(error)) from None
