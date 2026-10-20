#!/usr/bin/env python3
"""Publish checked packages through GitHub CLI; failures leave an unpublished draft.

No overwrite/recovery API or tag creation. Maintainers serialize publication for a
version; tag checks detect changes but do not lock administrators out of GitHub.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from candidate import REPOSITORY, arguments, validate_local
from package import TARGETS, sha

MAX_ASSET = 1024 * 1024 * 1024


def gh(*args):
    return subprocess.check_output(["gh", *args], text=True, timeout=600).strip()


def assets(directory, commit, tag):
    """Bind every attached byte to the two platform build receipts."""
    directory = Path(directory)
    files = {}
    for path in directory.rglob("*"):
        if path.is_symlink():
            raise ValueError("Symlink in release inputs")
        if path.is_file():
            if not re.fullmatch(r"[a-zA-Z0-9._-]+", path.name) or path.name in files:
                raise ValueError("Unsafe or duplicate asset name")
            if path.stat().st_size > MAX_ASSET:
                raise ValueError("Asset too large")
            files[path.name] = path
    expected, targets = {}, set()
    for name, path in files.items():
        if not name.endswith(".provenance.json"):
            continue
        if path.stat().st_size > 1024 * 1024:
            raise ValueError("Oversized provenance")
        report = json.loads(path.read_text())
        target = report.get("target")
        if (report.get("kind") != "development" or report.get("version") != tag[1:]
                or report.get("source", {}).get("commit") != commit
                or target not in TARGETS or target in targets):
            raise ValueError("Source/version/target mismatch")
        targets.add(target)
        for asset, record in report["assets"].items():
            if asset in expected or asset not in files:
                raise ValueError("Duplicate or missing declared asset")
            actual = files[asset]
            if actual.stat().st_size != record["bytes"] or sha(actual) != record["sha256"]:
                raise ValueError("Artifact does not match build receipt")
            expected[asset] = record["sha256"]
        expected[name] = sha(path)
        sums = name.removesuffix(".provenance.json") + ".SHA256SUMS"
        if sums not in files or files[sums].stat().st_size > 65536:
            raise ValueError("Missing checksum manifest")
        recorded = {}
        for line in files[sums].read_text().splitlines():
            match = re.fullmatch(r"([a-f0-9]{64})  ([a-zA-Z0-9._-]+)", line)
            if not match or match[2] in recorded:
                raise ValueError("Malformed checksum manifest")
            recorded[match[2]] = match[1]
        names = set(report["assets"]) | {name}
        if set(recorded) != names or any(recorded[n] != expected[n] for n in names):
            raise ValueError("Checksum manifest mismatch")
        expected[sums] = sha(files[sums])
    if targets != set(TARGETS) or set(files) != set(expected):
        raise ValueError("Exactly both target packages and their declared files are required")
    return files, expected


def remote_tag(commit, tag, command=gh):
    ref = json.loads(command("api", f"repos/{REPOSITORY}/git/ref/tags/{tag}"))["object"]
    for _ in range(8):
        if ref["type"] == "commit":
            if ref["sha"] != commit:
                raise ValueError("Remote tag moved from the built commit")
            return
        if ref["type"] != "tag" or not re.fullmatch(r"[a-f0-9]{40}", ref["sha"]):
            break
        ref = json.loads(command("api", f"repos/{REPOSITORY}/git/tags/{ref['sha']}"))["object"]
    raise ValueError("Unsupported tag chain")


def publish(directory, commit, tag, notes, command=gh):
    arguments(commit, tag)
    files, expected = assets(directory, commit, tag)
    # Copy the verified bytes to a private snapshot before the first mutation.
    with tempfile.TemporaryDirectory(prefix="nmt-publish-") as temp:
        temp = Path(temp)
        upload, download = temp / "upload", temp / "download"
        upload.mkdir()
        for name, original in files.items():
            shutil.copyfile(original, upload / name)
            if sha(upload / name) != expected[name]:
                raise ValueError("Input changed during snapshot")
        note = temp / "notes.md"
        note.write_text(Path(notes).read_text())
        remote_tag(commit, tag, command)
        command("release", "create", tag, "--repo", REPOSITORY, "--verify-tag", "--target", commit,
                "--draft", "--prerelease", "--title", f"NextCloud Modern Talk {tag} Alpha", "--notes-file", str(note))
        command("release", "upload", tag, *[str(upload / name) for name in sorted(files)], "--repo", REPOSITORY)
        command("release", "download", tag, "--repo", REPOSITORY, "--dir", str(download))
        if {p.name for p in download.iterdir()} != set(expected):
            raise ValueError("Downloaded release asset set differs; draft retained")
        for path in download.iterdir():
            if not path.is_file() or path.is_symlink() or sha(path) != expected[path.name]:
                raise ValueError("Downloaded bytes differ; draft retained")
        remote_tag(commit, tag, command)
        command("release", "edit", tag, "--repo", REPOSITORY, "--draft=false", "--prerelease", "--latest=false")
        return f"https://github.com/{REPOSITORY}/releases/tag/{tag}"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--notes", type=Path, required=True)
    parser.add_argument("--publish", action="store_true")
    args = parser.parse_args()
    validate_local(args.commit, args.tag)
    assets(args.directory, args.commit, args.tag)
    if args.publish:
        print(publish(args.directory, args.commit, args.tag, args.notes))
    else:
        print("Packages verified; no GitHub mutation requested.")
