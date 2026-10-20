#!/usr/bin/env python3
"""Install official pinned GStreamer SDKs for native compilation, never run the app.

Checksums come from the adjacent upstream .sha256sum files. Linux uses the
Ubuntu packages installed by tools/ui/prepare-linux.sh instead.
"""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import urllib.request

PACKAGES = {
    "aarch64-apple-darwin": (
        "https://gstreamer.freedesktop.org/data/pkg/osx/1.28.4/",
        [("gstreamer-1.0-1.28.4-universal.pkg", "9f9ae8ba1a25eb42d6740c4e5f15b09ee6b9127b7359e9662c45a2b9a5029f85"),
         ("gstreamer-1.0-devel-1.28.4-universal.pkg", "2758abb2abe10b79ad4cf17cbcf0a75e3cb71ee8ce01724960b8f3e0b715e065")]),
    "x86_64-pc-windows-msvc": (
        "https://gstreamer.freedesktop.org/data/pkg/windows/1.26.11/msvc/",
        [("gstreamer-1.0-msvc-x86_64-1.26.11.msi", "31cbc21fa0950b5c1e79c80959b2799805cb05a7a35953a13a9f790776137605"),
         ("gstreamer-1.0-devel-msvc-x86_64-1.26.11.msi", "af0a0720692052aaf5fb0cf2847ef34a87016ef0f914328d516f049e6c86f587")]),
}


def install(target):
    mac = target == "aarch64-apple-darwin"
    if platform.system() != ("Darwin" if mac else "Windows"):
        raise ValueError("Install only on the matching native build host")
    install_dir = Path("/Library/Frameworks/GStreamer.framework/Versions/1.0") if mac else Path(os.environ["RUNNER_TEMP"]) / "gstreamer"
    # Cerbero 1.26.11 adds Version/Architecture below INSTALLDIR; SDKROOTDIR
    # is the resulting SDK prefix, not INSTALLDIR itself. See upstream:
    # https://gitlab.freedesktop.org/gstreamer/cerbero/-/blob/1.26.11/cerbero/packages/wix.py#L581
    destination = install_dir if mac else install_dir / "1.0" / "msvc_x86_64"
    log_dir = None
    if not mac:
        # Keep MSI logs outside the temporary download directory on failure.
        log_dir = Path(os.environ["RUNNER_TEMP"]) / "gstreamer-installer-logs"
        log_dir.mkdir(parents=True, exist_ok=True)
    url, packages = PACKAGES[target]
    with tempfile.TemporaryDirectory(prefix="gstreamer-sdk-") as temp:
        for name, expected in packages:
            path = Path(temp) / name
            digest, total = hashlib.sha256(), 0
            with urllib.request.urlopen(url + name, timeout=60) as response, path.open("xb") as output:
                while data := response.read(1024 * 1024):
                    total += len(data)
                    if total > 1024 * 1024 * 1024:
                        raise ValueError("SDK archive too large")
                    digest.update(data); output.write(data)
            if digest.hexdigest() != expected:
                raise ValueError("GStreamer SDK checksum mismatch")
            if mac:
                command = ["sudo", "installer", "-pkg", str(path), "-target", "/"]
            else:
                log_path = log_dir / (name + ".log")
                command = ["msiexec", "/i", str(path), "/qn", "/norestart", "/L*v", str(log_path),
                           "ADDLOCAL=ALL", "INSTALLDIR=" + str(install_dir)]
                print(f"Installing {name}; SDK root: {destination}; MSI log: {log_path}", flush=True)
            result = subprocess.run(command, timeout=900)
            if result.returncode not in (0, 3010):
                detail = "" if mac else f"; MSI log: {log_path}"
                raise RuntimeError(f"GStreamer SDK installer {name} exited {result.returncode}{detail}")
    binary = destination / "bin"
    pkg_config = destination / "lib/pkgconfig/gstreamer-1.0.pc"
    if not pkg_config.is_file():
        detail = "" if mac else f"; MSI logs: {log_dir}"
        raise ValueError(f"Installed SDK has no GStreamer pkg-config file at {pkg_config}{detail}")
    with Path(os.environ["GITHUB_PATH"]).open("a") as output:
        output.write(str(binary) + "\n")
    with Path(os.environ["GITHUB_ENV"]).open("a") as output:
        output.write("PKG_CONFIG_PATH=" + str(destination / "lib/pkgconfig") + "\n")
        output.write("GSTREAMER_ROOT_X86_64=" + str(destination) + "\n")
    print("Installed verified GStreamer SDK at", destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=PACKAGES, required=True)
    install(parser.parse_args().target)
