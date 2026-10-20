#!/usr/bin/env python3
"""Record the native build host and require the target's compiler prerequisites.

Detection only: this never builds, tests or launches project code. With
--install-metal-toolchain it runs Apple's documented
`xcodebuild -downloadComponent metalToolchain` when the Xcode Metal compiler
cannot execute; GPUI compiles its Metal shaders in its build script.
This helper inspects the build host; it never executes the application or its tests.
"""
import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess

NATIVE = {
    "x86_64-unknown-linux-gnu": ("Linux", "x86_64"),
    "x86_64-pc-windows-msvc": ("Windows", "AMD64"),
    "aarch64-apple-darwin": ("Darwin", "arm64"),
}
VSWHERE = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
WINDOWS_SDK_LIBRARIES = Path(r"C:\Program Files (x86)\Windows Kits\10\Lib")


def run(argv, timeout=120):
    result = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    print("$ " + " ".join(argv) + (f"  (exit {result.returncode})" if result.returncode else ""), flush=True)
    output = (result.stdout + result.stderr).strip()
    if output:
        print(output, flush=True)
    return result


def require(result, message):
    if result.returncode != 0 or not result.stdout.strip():
        raise SystemExit(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=sorted(NATIVE), required=True)
    parser.add_argument("--install-metal-toolchain", action="store_true")
    args = parser.parse_args()
    # Labels can resolve to newer images or larger machines; log what actually ran.
    print(f"target {args.target}; host {platform.system()} {platform.machine()}; {platform.platform()}; "
          f"{os.cpu_count()} logical CPUs; Python {platform.python_version()}", flush=True)
    for key in ("ImageOS", "ImageVersion", "RUNNER_ENVIRONMENT"):
        if os.environ.get(key):
            print(f"{key}={os.environ[key]}", flush=True)
    if (platform.system(), platform.machine()) != NATIVE[args.target]:
        raise SystemExit(args.target + " must be compiled natively on " + " ".join(NATIVE[args.target]))

    if args.target.endswith("-linux-gnu"):
        release = Path("/etc/os-release")
        if release.is_file():
            print(next((line for line in release.read_text().splitlines() if line.startswith("PRETTY_NAME=")), "PRETTY_NAME unknown"))
    elif args.target.endswith("-windows-msvc"):
        require(run([VSWHERE, "-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                     "-property", "installationPath"]), "MSVC x64 build tools are required")
        sdks = sorted(path.name for path in WINDOWS_SDK_LIBRARIES.iterdir()) if WINDOWS_SDK_LIBRARIES.is_dir() else []
        print("Windows SDK libraries: " + (", ".join(sdks) or "none found"), flush=True)
        nasm = shutil.which("nasm")
        print("NASM: " + (nasm or "not found"), flush=True)
        # aws-lc-sys assembles with NASM on x86_64 Windows unless its prebuilt objects are allowed.
        if not nasm and os.environ.get("AWS_LC_SYS_PREBUILT_NASM") != "1":
            raise SystemExit("aws-lc-sys needs NASM or AWS_LC_SYS_PREBUILT_NASM=1 on x86_64 Windows")
    else:
        require(run(["sw_vers"]), "sw_vers failed")
        require(run(["xcodebuild", "-version"]), "Full Xcode is required")
        require(run(["xcrun", "--sdk", "macosx", "--show-sdk-version"]), "The macOS SDK is required")
        # Xcode 26 can locate a metal shim while its optional toolchain component is absent.
        if run(["xcrun", "--sdk", "macosx", "metal", "--version"]).returncode != 0:
            if not args.install_metal_toolchain:
                raise SystemExit("The Xcode Metal Toolchain is not installed")
            if run(["xcodebuild", "-downloadComponent", "metalToolchain"], timeout=1800).returncode != 0:
                raise SystemExit("Metal Toolchain download failed")
            require(run(["xcrun", "--sdk", "macosx", "metal", "--version"]), "The Metal compiler is still unavailable")
        require(run(["xcrun", "--sdk", "macosx", "--find", "metal"]), "metal not found")
        require(run(["xcrun", "--sdk", "macosx", "--find", "metallib"]), "metallib not found")


if __name__ == "__main__":
    main()
