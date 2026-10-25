#!/usr/bin/env python3
"""Build/run locally; macOS development uses one persistent signing identity."""
import json
import os
from pathlib import Path
import platform
import re
import shlex
import stat
import subprocess
import sys
import tempfile

import cargo

ROOT = Path(__file__).resolve().parents[1]
IDENTIFIER = "org.adapt2move.nextcloud-modern-talk.development"
CERTIFICATE = "NextCloud Modern Talk Local Development"


def run(argv, **kwargs):
    """Keep keychain paths/provider output private; never invoke a shell."""
    result = subprocess.run(list(map(str, argv)), capture_output=True, text=True,
                            timeout=120, **kwargs)
    if result.returncode:
        raise ValueError(f"{Path(str(argv[0])).name} {argv[1]} failed (exit {result.returncode})")
    return result.stdout.strip()


def private_root():
    directory = Path.home() / "Library/Application Support/NextCloud Modern Talk Development"
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    metadata = directory.lstat()
    if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.getuid()
            or stat.S_IMODE(metadata.st_mode) != 0o700):
        raise ValueError("Development signing directory must be an owner-only directory")
    return directory


def identity(directory):
    path = directory / "signing.json"
    if not path.exists():
        return None
    metadata = path.lstat()
    if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid()
            or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_size > 4096):
        raise ValueError("Invalid private development signing configuration")
    value = json.loads(path.read_text())
    fingerprint = value.get("certificate_sha1", "")
    if (value.get("schema") != 1 or value.get("identifier") != IDENTIFIER
            or re.fullmatch(r"[A-F0-9]{40}", fingerprint) is None):
        raise ValueError("Invalid development signing identity")
    identities = run(["security", "find-identity", "-v", "-p", "codesigning"])
    if fingerprint not in identities:
        raise ValueError("The configured signing identity is unavailable; restore it in Keychain Access")
    return fingerprint


def setup_signing(directory=None):
    if platform.system() != "Darwin":
        raise ValueError("Local development signing is only needed on macOS")
    directory = directory or private_root()
    existing = identity(directory)
    if existing:
        print("Local development signing is already configured.", flush=True)
        return existing
    # Reuse a prior successful import if configuration writing was interrupted.
    available = run(["security", "find-identity", "-p", "codesigning"])
    matches = re.findall(r'\d+\) ([A-F0-9]{40}) "' + re.escape(CERTIFICATE) + r'"', available)
    if len(set(matches)) > 1:
        raise ValueError("Multiple local development certificates exist; resolve them in Keychain Access")
    if matches:
        fingerprint = matches[0]
    else:
        print("Creating one local code-signing certificate in your user keychain.", flush=True)
        keychain = shlex.split(run(["security", "default-keychain", "-d", "user"]))
        if len(keychain) != 1 or not Path(keychain[0]).is_absolute():
            raise ValueError("No default user keychain is available")
        with tempfile.TemporaryDirectory(prefix="signing-", dir=directory) as temporary:
            stage = Path(temporary)
            config = stage / "request.cnf"
            key = stage / "private.pem"
            certificate = stage / "certificate.pem"
            config.write_text("[req]\nprompt=no\ndistinguished_name=subject\nx509_extensions=usage\n"
                              f"[subject]\nCN={CERTIFICATE}\n[usage]\n"
                              "basicConstraints=critical,CA:FALSE\n"
                              "keyUsage=critical,digitalSignature\nextendedKeyUsage=codeSigning\n")
            previous = os.umask(0o077)
            try:
                run(["openssl", "req", "-x509", "-newkey", "rsa:3072", "-sha256", "-nodes",
                     "-days", "3650", "-config", config, "-keyout", key, "-out", certificate])
            finally:
                os.umask(previous)
            digest = run(["openssl", "x509", "-in", certificate, "-noout", "-fingerprint", "-sha1"])
            fingerprint = digest.split("=", 1)[-1].replace(":", "").strip().upper()
            if re.fullmatch(r"[A-F0-9]{40}", fingerprint) is None:
                raise ValueError("Could not identify the generated certificate")
            # Non-extractable key; only Apple's signing tool gets standing access.
            # No credential item ACL or TLS/system trust is changed.
            run(["security", "import", key, "-k", keychain[0], "-t", "priv",
                 "-x", "-T", "/usr/bin/codesign"])
            run(["security", "add-trusted-cert", "-r", "trustRoot", "-p", "codeSign",
                 "-k", keychain[0], certificate])
    config = {"schema": 1, "identifier": IDENTIFIER, "certificate_sha1": fingerprint}
    path = directory / "signing.json"
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as file:
        json.dump(config, file)
        file.flush()
        os.fsync(file.fileno())
    return identity(directory)


def sign(binary, directory=None):
    directory = directory or private_root()
    signer = identity(directory)
    if signer is None:
        raise ValueError("Run 'task dev:setup' once to configure stable macOS development signing")
    run(["codesign", "--force", "--sign", signer, "--identifier", IDENTIFIER,
         "--timestamp=none", binary])
    run(["codesign", "--verify", "--strict", binary])


def main(argv=None):
    args = list(sys.argv[1:] if argv is None else argv)
    try:
        if args == ["--setup-signing"]:
            setup_signing()
            print("Stable development signing is ready. Start with 'task dev'.", flush=True)
            return 0
        if platform.system() != "Darwin":
            return cargo.main(["run", "--locked", "-p", "talk-desktop", "--", *args])
        # Fail before a build if setup is needed; never silently fall back to a
        # fresh ad-hoc identity and another cycle of permission prompts.
        if identity(private_root()) is None:
            raise ValueError("Run 'task dev:setup' once before 'task dev' on macOS")
        subprocess.run([sys.executable, str(ROOT / "tools/cargo.py"), "build", "--locked",
                        "-p", "talk-desktop"], cwd=ROOT, check=True)
        binary = ROOT / "target/debug/talk-desktop"
        sign(binary)
        os.chdir(ROOT)
        os.execve(binary, [str(binary), *args], cargo.environment(["run"]))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Development: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
