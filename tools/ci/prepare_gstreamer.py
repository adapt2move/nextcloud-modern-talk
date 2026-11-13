#!/usr/bin/env python3
"""Install official pinned GStreamer SDKs for native compilation, never run the app.

Checksums come from the adjacent upstream .sha256sum files. Linux uses the
Ubuntu packages installed by tools/ui/prepare-linux.sh instead.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import http.client
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time
import urllib.error
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

# The complete development SDK supports production codecs/block formation and
# opt-in webrtcbin integration probes. Presence is a layout check, not proof of
# plugin loading, ICE, DTLS or TLS certificate verification on a native host.
REQUIRED_PC = ("gstreamer-1.0", "gstreamer-app-1.0", "gstreamer-audio-1.0",
               "gstreamer-video-1.0", "gstreamer-rtp-1.0", "gstreamer-net-1.0",
               "gstreamer-sdp-1.0", "gstreamer-webrtc-1.0")
REQUIRED_PLUGINS = ("app", "opus", "vpx", "audioconvert", "audioresample",
                    "audiobuffersplit", "rtp", "rtpmanager", "webrtc", "nice", "dtls", "srtp")


def verify_sdk(destination, mac):
    required = [destination / "lib/pkgconfig" / (name + ".pc") for name in REQUIRED_PC]
    prefix, suffix = ("libgst", ".dylib") if mac else ("gst", ".dll")
    required += [destination / "lib/gstreamer-1.0" / (prefix + name + suffix)
                 for name in REQUIRED_PLUGINS]
    for path in required:
        if not path.is_file():
            raise ValueError(f"Installed SDK is missing required media input: {path}")


MAX_RANGE_BYTES = 16 * 1024 * 1024


class IncompleteDownload(ValueError):
    """A correctly identified response ended before all expected bytes arrived."""


def retry_transfer(operation, deadline, attempts=3):
    """A few attempts within the original deadline; never retry invalid content."""
    for attempt in range(attempts):
        if time.monotonic() >= deadline:
            raise ValueError("SDK download exceeded its time limit")
        delay = 2 ** min(attempt, 4)
        try:
            return operation()
        except urllib.error.HTTPError as error:
            retry_after = (error.headers or {}).get("Retry-After")
            error.close()
            # Rate limits and permanent errors remain explicit failures. A date
            # Retry-After also fails rather than retrying before an unknown wait.
            if error.code not in (408, 500, 502, 503, 504):
                raise
            if retry_after is not None:
                if not retry_after.strip().isascii() or not retry_after.strip().isdigit():
                    raise
                delay = max(delay, int(retry_after))
            if attempt == attempts - 1 or time.monotonic() + delay >= deadline:
                raise
        except (IncompleteDownload, http.client.IncompleteRead, urllib.error.URLError,
                TimeoutError, ConnectionError):
            if attempt == attempts - 1 or time.monotonic() + delay >= deadline:
                raise
        time.sleep(delay)


def download(url, destination, expected):
    """Fetch at most four concurrent 16 MiB ranges, then verify the pinned SDK.

    The upstream server advertises byte ranges; its large single responses can
    stall long enough to consume a native build's lifetime. Smaller servers and
    endpoints without range support retain a single streaming download.
    """
    limit, deadline = 1024 * 1024 * 1024, time.monotonic() + 1200
    def probe():
        with urllib.request.urlopen(urllib.request.Request(url, method="HEAD"), timeout=60) as response:
            headers = getattr(response, "headers", {})
            try:
                size = int(headers.get("Content-Length", ""))
            except ValueError:
                size = None
            ranges = headers.get("Accept-Ranges", "").strip().lower() == "bytes"
            return size, ranges
    size, ranges = None, False
    try:
        size, ranges = retry_transfer(probe, deadline)
    except urllib.error.HTTPError as error:
        if error.code not in (405, 501):
            raise
    if size is not None and size > limit:
        raise ValueError("SDK archive too large")
    parallel = ranges and size is not None and size >= 8 * 1024 * 1024
    print(f"Downloading {destination.name}: {size or 'unknown'} bytes, {4 if parallel else 1} transfer(s)", flush=True)

    def copy(source, output, maximum, digest=None):
        total = 0
        while chunk := source.read(64 * 1024):
            total += len(chunk)
            if total > maximum or time.monotonic() >= deadline:
                raise ValueError("SDK download exceeded its size or time limit")
            output.write(chunk)
            if digest is not None:
                digest.update(chunk)
        return total

    digest, created = hashlib.sha256(), False
    try:
        if parallel:
            with tempfile.TemporaryDirectory(prefix="gstreamer-ranges-", dir=destination.parent) as temporary:
                width = min(MAX_RANGE_BYTES, (size + 3) // 4)
                # The 1 GiB ceiling bounds this list and the executor's queue to
                # at most 64 parts. Each part owns one file and at most 3 attempts.
                parts = [Path(temporary) / str(index) for index in range((size + width - 1) // width)]
                def fetch(index):
                    start, end = index * width, min((index + 1) * width, size) - 1
                    wanted, part = end - start + 1, parts[index]
                    def transfer():
                        # The mirror cuts some ranges at a fixed offset, so repeating
                        # the same range repeats the cut. Keep what arrived and ask
                        # only for the rest; the final checksum still covers it all.
                        have = part.stat().st_size if part.exists() else 0
                        first = start + have
                        request = urllib.request.Request(url, headers={"Range": f"bytes={first}-{end}"})
                        try:
                            with urllib.request.urlopen(request, timeout=60) as response:
                                if response.status != 206 or response.headers.get("Content-Range", "").strip() != f"bytes {first}-{end}/{size}":
                                    raise ValueError("SDK server returned an unexpected byte range")
                                with part.open("ab") as output:
                                    copy(response, output, wanted - have)
                        except (http.client.IncompleteRead, ConnectionError, TimeoutError):
                            pass  # keep the bytes that arrived; the size check below decides
                        if part.stat().st_size != wanted:
                            raise IncompleteDownload("SDK server returned an incomplete byte range")
                    retry_transfer(transfer, deadline, attempts=8)
                with ThreadPoolExecutor(max_workers=4) as pool:
                    list(pool.map(fetch, range(len(parts))))
                with destination.open("xb") as output:
                    created = True
                    for index, part in enumerate(parts):
                        with part.open("rb") as source:
                            copy(source, output, min(width, size - index * width), digest)
        else:
            def stream():
                nonlocal created, digest
                digest = hashlib.sha256()
                try:
                    with urllib.request.urlopen(url, timeout=60) as response, destination.open("xb") as output:
                        created = True
                        received = copy(response, output, limit, digest)
                        if size is not None and size > 0 and received != size:
                            raise IncompleteDownload("SDK server returned an incomplete download")
                except BaseException:
                    if created:
                        destination.unlink(missing_ok=True)
                        created = False
                    raise
            retry_transfer(stream, deadline)
        actual = digest.hexdigest()
        if actual != expected:
            raise ValueError(f"GStreamer SDK checksum mismatch for {destination.name}: expected {expected}, received {actual}")
    except BaseException:
        if created:
            destination.unlink(missing_ok=True)
        raise
    print("Verified SDK archive:", destination.name, flush=True)


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
            download(url + name, path, expected)
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
    verify_sdk(destination, mac)
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
