#!/usr/bin/env python3
"""Fetch the exact maintained libwebrtc artifact before Cargo's build script.

Pass the printed directory as LK_CUSTOM_WEBRTC. Digests are GitHub's release
asset digests; they identify bytes and are not a publisher-signature claim.
The crate's automatic, unchecked download is deliberately not used by releases.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import tempfile
import urllib.request
import zipfile

TAG = "webrtc-89d790b"
ARTIFACTS = {
    "linux-x64-release": (
        156139058,
        "b167adad5291cea0e4d66a0454d9d52d2ad714e6b0ed70f4410317d3ebde70c5",
    ),
    "win-x64-release": (
        114396365,
        "5c2349c960bae4f06f71c102f58552d0d09c912a142811acfcadfb7c883cbf58",
    ),
    "mac-arm64-release": (
        277148750,
        "9f25fea48588deac18d68e120d18b33af7ef10f921f74b7c57f66b642262c348",
    ),
}


def digest(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def prepare(target, output):
    length, sha256 = ARTIFACTS[target]
    output = output.resolve()
    final = output / f"{TAG}-{target}"
    marker = final / ".nmt-native.json"
    if final.exists():
        record = json.loads(marker.read_text())
        if record["archive_sha256"] != sha256 or record["target"] != target:
            raise ValueError("Existing native artifact identity differs")
        actual = {str(p.relative_to(final)) for p in final.rglob("*") if p.is_file() or p.is_symlink()}
        if actual != set(record["files"]) | {".nmt-native.json"}:
            raise ValueError("Existing native artifact file inventory differs")
        for relative, expected in record["files"].items():
            path = final / relative
            if path.is_symlink() or not path.is_file() or digest(path) != expected:
                raise ValueError("Existing native artifact was modified")
        return final
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".native-", dir=output) as scratch:
        scratch = Path(scratch)
        archive = scratch / "archive.zip"
        url = f"https://github.com/livekit/rust-sdks/releases/download/{TAG}/webrtc-{target}.zip"
        request = urllib.request.Request(url, headers={"User-Agent": "nextcloud-modern-talk-build"})
        size = 0
        with urllib.request.urlopen(request, timeout=45) as response, archive.open("xb") as destination:
            while chunk := response.read(1024 * 1024):
                size += len(chunk)
                if size > length:
                    raise ValueError("Native artifact exceeds pinned size")
                destination.write(chunk)
        if size != length or digest(archive) != sha256:
            raise ValueError("Native artifact digest or size differs")
        unpacked = scratch / "unpacked"
        unpacked.mkdir()
        files = {}
        total = 0
        with zipfile.ZipFile(archive) as source:
            entries = source.infolist()
            if len(entries) > 100_000:
                raise ValueError("Too many native archive entries")
            seen = set()
            for entry in entries:
                path = PurePosixPath(entry.filename)
                if (path.is_absolute() or ".." in path.parts or "\\" in entry.filename
                        or not path.parts or path.parts[0] != target or path in seen):
                    raise ValueError("Invalid or duplicate archive path")
                seen.add(path)
                mode = entry.external_attr >> 16
                if mode & 0o170000 == 0o120000:
                    raise ValueError("Native archive symlinks are not admitted")
                total += entry.file_size
                if entry.file_size > 1024**3 or total > 4 * 1024**3:
                    raise ValueError("Native archive expansion exceeds build budget")
                destination = unpacked.joinpath(*path.parts)
                if entry.is_dir():
                    destination.mkdir(parents=True, exist_ok=True)
                    continue
                destination.parent.mkdir(parents=True, exist_ok=True)
                with source.open(entry) as data, destination.open("xb") as handle:
                    shutil.copyfileobj(data, handle, 1024 * 1024)
                destination.chmod(0o644)
                files[str(path.relative_to(target))] = digest(destination)
        native = unpacked / target
        library = "lib/webrtc.lib" if target.startswith("win-") else "lib/libwebrtc.a"
        if not (native / "include").is_dir() or not (native / library).is_file():
            raise ValueError("Native archive is missing headers or engine library")
        (native / ".nmt-native.json").write_text(json.dumps({
            "tag": TAG, "target": target, "archive_sha256": sha256,
            "archive_bytes": length, "files": files,
        }, sort_keys=True) + "\n")
        native.rename(final)
    return final


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=ARTIFACTS, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(prepare(args.target, args.output))


if __name__ == "__main__":
    main()
