#!/usr/bin/env python3
"""Create/reuse one private alpha signer and optionally configure GitHub Actions.

This identity provides continuity, not Developer ID or notarization. Never run
it on a release runner: the encrypted backup and password must survive releases.
Existing repository signing configuration is never overwritten.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import tempfile

CERTIFICATE = "NextCloud Modern Talk Alpha"


def run(command, *, data=None):
    result = subprocess.run([str(c) for c in command], input=data, capture_output=True, timeout=60)
    if result.returncode:
        raise ValueError("Signing setup command failed: " + str(command[0]) + " (credentials and provider output withheld)")
    return result.stdout


def private_file(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as file:
        file.write(data)
        file.flush()
        os.fsync(file.fileno())


def inspect(path, *, directory=False):
    info = path.lstat()
    if (not (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
            or info.st_mode & 0o077 or info.st_uid != os.getuid()):
        raise ValueError("Signing backup must contain owner-private regular files in an owner-private directory")


def prepare(directory):
    directory = directory.absolute()
    if not directory.exists():
        directory.mkdir(mode=0o700, parents=False)
    inspect(directory, directory=True)
    config = directory / "identity.json"
    if not config.exists():
        if any(directory.iterdir()):
            raise ValueError("Refusing to replace an incomplete signing backup")
        with tempfile.TemporaryDirectory(prefix="create-", dir=directory) as temporary:
            stage = Path(temporary)
            password = stage / "password"
            private_file(password, secrets.token_urlsafe(48).encode())
            request = stage / "request.cnf"
            request.write_text("[req]\nprompt=no\ndistinguished_name=subject\nx509_extensions=usage\n"
                               "[subject]\nCN=" + CERTIFICATE + "\n[usage]\n"
                               "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\n"
                               "extendedKeyUsage=codeSigning\n")
            key, cert, p12 = stage / "key.pem", stage / "certificate.pem", stage / "identity.p12"
            previous = os.umask(0o077)
            try:
                run(["openssl", "req", "-x509", "-newkey", "rsa:3072", "-sha256", "-nodes", "-days", "3650",
                     "-config", request, "-keyout", key, "-out", cert])
                run(["openssl", "pkcs12", "-export", "-inkey", key, "-in", cert, "-name", CERTIFICATE,
                     "-keypbe", "PBE-SHA1-3DES", "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha256",
                     "-out", p12, "-passout", "file:" + str(password)])
            finally:
                os.umask(previous)
            digest = run(["openssl", "x509", "-in", cert, "-noout", "-fingerprint", "-sha1"])
            fingerprint = digest.decode().split("=", 1)[-1].replace(":", "").strip().upper()
            if not re.fullmatch(r"[A-F0-9]{40}", fingerprint):
                raise ValueError("Invalid signing certificate fingerprint")
            for source, name in ((p12, "identity.p12"), (cert, "certificate.pem"), (password, "password")):
                private_file(directory / name, source.read_bytes())
            private_file(config, (json.dumps({"schema": 1, "certificate_sha1": fingerprint}) + "\n").encode())
    for name in ("identity.json", "identity.p12", "certificate.pem", "password"):
        inspect(directory / name)
    value = json.loads(config.read_text())
    fingerprint = value.get("certificate_sha1", "")
    if value.get("schema") != 1 or not re.fullmatch(r"[A-F0-9]{40}", fingerprint):
        raise ValueError("Invalid signing backup")
    digest = run(["openssl", "x509", "-in", directory / "certificate.pem", "-noout", "-fingerprint", "-sha1"])
    if digest.decode().split("=", 1)[-1].replace(":", "").strip().upper() != fingerprint:
        raise ValueError("Signing backup certificate does not match its identity")
    embedded = run(["openssl", "pkcs12", "-in", directory / "identity.p12", "-passin",
                    "file:" + str(directory / "password"), "-clcerts", "-nokeys"])
    digest = run(["openssl", "x509", "-noout", "-fingerprint", "-sha1"], data=embedded)
    if digest.decode().split("=", 1)[-1].replace(":", "").strip().upper() != fingerprint:
        raise ValueError("Encrypted signing identity does not match its certificate")
    return directory, fingerprint


def install(directory, fingerprint, repository):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("Invalid repository")
    secret_names = {s["name"] for s in json.loads(run(["gh", "secret", "list", "--repo", repository, "--json", "name"]))}
    variables = {v["name"]: v["value"] for v in json.loads(run(["gh", "variable", "list", "--repo", repository, "--json", "name,value"]))}
    expected = {"MACOS_SIGNING_MODE": "internal", "MACOS_SIGNING_CERTIFICATE": fingerprint}
    names = {"MACOS_CERTIFICATE_P12_BASE64", "MACOS_CERTIFICATE_PASSWORD"}
    existing = {name: value for name, value in variables.items() if name.startswith("MACOS_")}
    if names <= secret_names and existing == expected:
        return
    if any(name.startswith("MACOS_") for name in secret_names) or existing:
        raise ValueError("Repository signing configuration already exists; refusing to rotate or overwrite it")
    for name, value in (("MACOS_CERTIFICATE_P12_BASE64", base64.b64encode((directory / "identity.p12").read_bytes())),
                        ("MACOS_CERTIFICATE_PASSWORD", (directory / "password").read_bytes())):
        run(["gh", "secret", "set", name, "--repo", repository], data=value)
    for name, value in expected.items():
        run(["gh", "variable", "set", name, "--repo", repository, "--body", value])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--install-repository")
    args = parser.parse_args()
    directory, fingerprint = prepare(args.directory)
    if args.install_repository:
        install(directory, fingerprint, args.install_repository)
    print("Persistent internal alpha certificate: " + fingerprint)
    print("Keep the owner-private signing backup at: " + str(directory))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        raise SystemExit(str(error)) from None
