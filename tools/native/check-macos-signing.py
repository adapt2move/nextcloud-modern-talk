#!/usr/bin/env python3
"""Real macOS signing contract: two app versions, one disposable internal signer.

Does not run the app, request OS permissions, install trust or change the default
keychain. It proves native import/sealing/requirement continuity, not TCC behavior.
"""
import base64
import importlib.util
import os
from pathlib import Path
import platform
import plistlib
import subprocess
import sys
import tempfile
import shlex

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/release"))
import internal_identity
import macos_signing
import macos_runtime


def code_command(arguments):
    # This lane only invokes codesign with public fixture paths, certificate
    # hashes and requirements. Its diagnostics contain no import passwords or
    # decrypted keys; credential-bearing setup keeps its redacted error route.
    phase = "sign" if "--sign" in arguments else "verify"
    print("Native signing contract: " + phase, flush=True)
    print("Public fixture invocation: " + " ".join(map(str, arguments)), flush=True)
    try:
        return macos_runtime.run(arguments, timeout=30)
    except (ValueError, subprocess.SubprocessError) as error:
        # Preserve the primary public fixture failure if a later native cleanup
        # also fails or times out.
        print("Native " + phase + " failure: " + str(error), flush=True)
        raise


def check():
    if platform.system() != "Darwin":
        raise ValueError("This contract check requires macOS")
    spec = importlib.util.spec_from_file_location("ci_signing", ROOT / "tools/ci/macos_signing.py")
    ci = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ci)
    with tempfile.TemporaryDirectory(prefix="nmt-signing-contract-", dir=os.environ["RUNNER_TEMP"]) as temporary:
        root = Path(temporary)
        backup, fingerprint = internal_identity.prepare(root / "identity")
        os.environ.update({"MACOS_CERTIFICATE_P12_BASE64": base64.b64encode((backup / "identity.p12").read_bytes()).decode(),
            "MACOS_CERTIFICATE_PASSWORD": (backup / "password").read_text(), "MACOS_SIGNING_MODE": "internal",
            "MACOS_SIGNING_CERTIFICATE": fingerprint, "MACOS_SIGNING_TEAM": "", "GITHUB_ENV": str(root / "env")})
        search = shlex.split(ci.run(["security", "list-keychains", "-d", "user"]))
        try:
            ci.prepare()
            for line in (root / "env").read_text().splitlines():
                name, value = line.split("=", 1)
                os.environ[name] = value
            current = shlex.split(ci.run(["security", "list-keychains", "-d", "user"]))
            keychain = os.environ["NMT_MACOS_SIGNING_KEYCHAIN"]
            print("Temporary keychain in search list before signing: " + str(keychain in current), flush=True)
            if keychain not in current:
                ci.run(["security", "list-keychains", "-d", "user", "-s", *current, keychain])
            valid = ci.run(["security", "find-identity", "-v", "-p", "codesigning", os.environ["NMT_MACOS_SIGNING_KEYCHAIN"]])
            print("Fixture has system-trusted code-signing identity: " + str(fingerprint in valid), flush=True)
            requirements = []
            for version in (1, 2):
                app = root / ("Version" + str(version) + ".app")
                contents = app / "Contents"
                executable = contents / "MacOS/fixture"
                executable.parent.mkdir(parents=True)
                (contents / "Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": macos_signing.IDENTIFIER,
                    "CFBundleExecutable": "fixture", "CFBundlePackageType": "APPL", "CFBundleVersion": str(version)}))
                source = root / "fixture.c"
                source.write_text('int main(void) { return ' + str(version) + '; }\n')
                internal_identity.run(["clang", source, "-o", executable])
                macos_signing.sign(app, code_command, application=True)
                macos_signing.verify_application(app, code_command)
                result = subprocess.run(["codesign", "-d", "-r-", str(app)], capture_output=True, text=True, check=True, timeout=30)
                requirement = next((line for line in (result.stdout + result.stderr).splitlines() if line.startswith("designated =>")), "")
                if fingerprint.lower() not in requirement.lower() or macos_signing.IDENTIFIER not in requirement:
                    raise ValueError("The native designated requirement lost its certificate/identifier binding")
                requirements.append(requirement)
                wrong = dict(macos_signing.configuration(), identity="B" * 40)
                rejected = subprocess.run(["codesign", "--verify", "--strict", "-R=" + macos_signing.requirement(wrong, macos_signing.IDENTIFIER), str(app)], capture_output=True, timeout=30)
                if rejected.returncode == 0:
                    raise ValueError("Native verification accepted the wrong signer")
            if requirements[0] != requirements[1]:
                raise ValueError("The designated requirement changed between application versions")
        finally:
            try:
                if directory := os.environ.get("NMT_MACOS_SIGNING_TEMP"):
                    ci.cleanup(Path(directory))
            finally:
                try:
                    ci.run(["security", "list-keychains", "-d", "user", "-s", *search])
                finally:
                    for name in ("MACOS_CERTIFICATE_P12_BASE64", "MACOS_CERTIFICATE_PASSWORD", "NMT_MACOS_SIGNING_TEMP"):
                        os.environ.pop(name, None)
    print("Native internal-signing import, version continuity and wrong-signer rejection passed; no OS permission qualification.")


if __name__ == "__main__":
    try:
        check()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(str(error)) from None
