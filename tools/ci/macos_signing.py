#!/usr/bin/env python3
"""Import a persistent release identity into one disposable runner keychain.

The disposable keychain is appended to the runner's search list and the exact
original list is restored on cleanup. No default keychain/trust changes,
credential output or generated release identity.
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
import json
import shlex


def checked_search_list(value):
    if (not isinstance(value, list) or len(value) > 32
            or any(not isinstance(path, str) or not Path(path).is_absolute()
                   or len(path) > 4096 or any(c in path for c in "\r\n\0") for path in value)):
        raise ValueError("Invalid macOS keychain search list")
    return value


def search_list():
    value = run(["security", "list-keychains", "-d", "user"])
    if len(value.encode()) > 16384:
        raise ValueError("macOS keychain search list exceeds its size limit")
    return checked_search_list(shlex.split(value))


def run(arguments):
    result = subprocess.run(arguments, capture_output=True, text=True, timeout=30)
    if result.returncode:
        # security's stderr may include certificate/keychain/provider details.
        operation = arguments[1] if len(arguments) > 1 else "unknown"
        raise ValueError("Temporary macOS signing keychain operation failed ("
                         + operation + ", exit " + str(result.returncode) + ")")
    return result.stdout


def compatible_identity(certificate, password):
    """Rewrap our internal P12 for Apple's legacy importer, preserving its key.

    The certificate still uses SHA-256/RSA. SHA-1 here is the password-container
    MAC, never the code signature or certificate signature. Private-key PEM stays
    in a pipe/in memory and never enters a file, workflow output or diagnostics.
    Developer ID imports keep their supplied distribution container unchanged.
    """
    destination = certificate.with_name("compatible.p12")
    with destination.open("xb"):
        destination.chmod(0o600)
    environment = dict(os.environ, NMT_P12_IMPORT_PASSWORD=password)
    decode = subprocess.run(["openssl", "pkcs12", "-in", str(certificate),
        "-passin", "env:NMT_P12_IMPORT_PASSWORD", "-nodes", "-nocerts"],
        capture_output=True, timeout=30, env=environment)
    if decode.returncode or not decode.stdout or len(decode.stdout) > 512 * 1024:
        raise ValueError("Internal signing container conversion failed (decode)")
    public = subprocess.run(["openssl", "pkcs12", "-in", str(certificate),
        "-passin", "env:NMT_P12_IMPORT_PASSWORD", "-nokeys"],
        capture_output=True, timeout=30, env=environment)
    if public.returncode or not public.stdout or len(public.stdout) > 256 * 1024:
        raise ValueError("Internal signing container conversion failed (certificate)")
    public_file = certificate.with_name("compatible-cert.pem")
    try:
        with public_file.open("xb") as output:
            public_file.chmod(0o600)
            output.write(public.stdout)
        # OpenSSL needs a seekable certificate input. Only the public part uses
        # a file; the private key is consumed separately from the input pipe.
        encode = subprocess.run(["openssl", "pkcs12", "-export", "-out", str(destination),
            "-in", str(public_file), "-inkey", "/dev/stdin",
            "-passout", "env:NMT_P12_IMPORT_PASSWORD", "-keypbe", "PBE-SHA1-3DES",
            "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha1", "-iter", "100000"],
            input=decode.stdout, capture_output=True, timeout=30, env=environment)
    finally:
        public_file.unlink(missing_ok=True)
    if encode.returncode or not destination.stat().st_size or destination.stat().st_size > 256 * 1024:
        raise ValueError("Internal signing container conversion failed (encode)")
    return destination


def credentials(env):
    encoded = env.get("MACOS_CERTIFICATE_P12_BASE64", "")
    password = env.get("MACOS_CERTIFICATE_PASSWORD", "")
    team = env.get("MACOS_SIGNING_TEAM", "")
    mode = env.get("MACOS_SIGNING_MODE", "developer-id")
    pin = env.get("MACOS_SIGNING_CERTIFICATE", "")
    if not encoded or not password:
        raise ValueError("Publishing needs MACOS_CERTIFICATE_P12_BASE64 and MACOS_CERTIFICATE_PASSWORD")
    if mode == "developer-id":
        if not re.fullmatch(r"[A-Z0-9]{10}", team) or pin:
            raise ValueError("Developer ID requires MACOS_SIGNING_TEAM")
    elif mode != "internal" or team or not re.fullmatch(r"[A-F0-9]{40}", pin):
        raise ValueError("Internal signing requires MACOS_SIGNING_CERTIFICATE and no Apple team")
    if len(encoded) > 350_000:
        raise ValueError("macOS signing certificate exceeds its size limit")
    encoded = "".join(encoded.split())
    try:
        data = base64.b64decode(encoded, validate=True)
    except ValueError:
        raise ValueError("Invalid macOS signing certificate encoding") from None
    if not data or len(data) > 256 * 1024:
        raise ValueError("Invalid macOS signing certificate size")
    return data, password, team, mode, pin


def cleanup(directory):
    root = Path(os.environ["RUNNER_TEMP"]).resolve()
    directory = Path(directory)
    if (directory.is_symlink() or not directory.resolve().is_relative_to(root)
            or directory.parent.resolve() != root or not directory.name.startswith("nmt-signing-")):
        raise ValueError("Invalid temporary signing directory")
    keychain = directory / "release.keychain-db"
    failure = None
    snapshot = directory / "search-list.json"
    try:
        if snapshot.exists():
            if snapshot.is_symlink() or snapshot.stat().st_size > 16384:
                raise ValueError("Invalid keychain search-list snapshot")
            previous = checked_search_list(json.loads(snapshot.read_text()))
            run(["security", "list-keychains", "-d", "user", "-s", *previous])
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        failure = error
    try:
        if keychain.exists():
            run(["security", "delete-keychain", str(keychain)])
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        failure = failure or error
    if failure:
        # Keep the private snapshot for a later cleanup retry, but never claim
        # success or skip keychain deletion because list restoration failed.
        raise ValueError("Temporary macOS signing cleanup failed") from failure
    shutil.rmtree(directory)


def prepare():
    data, password, team, mode, pin = credentials(os.environ)
    directory = Path(tempfile.mkdtemp(prefix="nmt-signing-", dir=os.environ["RUNNER_TEMP"]))
    directory.chmod(0o700)
    keychain = directory / "release.keychain-db"
    certificate = directory / "identity.p12"
    try:
        previous = search_list()
        snapshot = json.dumps(previous, ensure_ascii=False)
        if len(snapshot.encode()) > 16384:
            raise ValueError("macOS keychain search-list snapshot exceeds its size limit")
        with (directory / "search-list.json").open("x") as output:
            (directory / "search-list.json").chmod(0o600)
            output.write(snapshot)
        with certificate.open("xb") as output:
            certificate.chmod(0o600)
            output.write(data)
        # This password protects only the temporary local keychain. It is not a
        # replacement release credential and never enters logs or GITHUB_ENV.
        local_password = secrets.token_hex(32)
        run(["security", "create-keychain", "-p", local_password, str(keychain)])
        run(["security", "set-keychain-settings", "-lut", "7200", str(keychain)])
        run(["security", "unlock-keychain", "-p", local_password, str(keychain)])
        imported = compatible_identity(certificate, password) if mode == "internal" else certificate
        run(["security", "import", str(imported), "-f", "pkcs12", "-k", str(keychain), "-P", password, "-T", "/usr/bin/codesign"])
        run(["security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", local_password, str(keychain)])
        identities = run(["security", "find-identity", *(["-v"] if mode == "developer-id" else []), "-p", "codesigning", str(keychain)])
        if mode == "developer-id":
            fingerprints = re.findall(r'\b([A-F0-9]{40}) "Developer ID Application: [^"\r\n]+ \(' + re.escape(team) + r'\)"', identities)
        else:
            # Untrusted self-signed roots are intentional here. No trust entry
            # is installed; codesign verifies the exact leaf after every seal.
            fingerprints = list(set(re.findall(r'\b([A-F0-9]{40}) "[^"\r\n]+"', identities)))
        if len(fingerprints) != 1 or mode == "internal" and fingerprints[0] != pin:
            raise ValueError("Exactly one matching persistent code-signing identity is required")
        # --keychain alone does not make an otherwise hidden identity available
        # to codesign. Keep the original order/default and add only our own lane.
        run(["security", "list-keychains", "-d", "user", "-s", *previous, str(keychain)])
        with Path(os.environ["GITHUB_ENV"]).open("a") as output:
            output.write("NMT_MACOS_REQUIRE_SIGNED=1\nNMT_MACOS_SIGNING_IDENTITY=" + fingerprints[0]
                         + "\nNMT_MACOS_SIGNING_TEAM=" + team + "\nNMT_MACOS_SIGNING_KEYCHAIN=" + str(keychain)
                         + "\nNMT_MACOS_SIGNING_MODE=" + mode + "\nNMT_MACOS_SIGNING_CERTIFICATE=" + pin
                         + "\nNMT_MACOS_SIGNING_TEMP=" + str(directory) + "\n")
    except BaseException:
        cleanup(directory)
        raise
    finally:
        certificate.unlink(missing_ok=True)
        certificate.with_name("compatible.p12").unlink(missing_ok=True)


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
