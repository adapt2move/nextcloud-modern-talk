#!/usr/bin/env python3
"""Offline publication failure checks. No GitHub operations."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from publish import assets, publish
from candidate import REPOSITORY
from package import sha, TARGETS

COMMIT = "a" * 40
TAG = "v0.0.1"


def fixture(root, tag=TAG):
    for target in TARGETS:
        name = target + ".tar.gz"
        (root / name).write_bytes(b"synthetic packaging fixture")
        record = {"kind": "development", "version": tag[1:], "target": target,
                  "source": {"commit": COMMIT}, "assets": {name: {"sha256": sha(root / name), "bytes": (root / name).stat().st_size}}}
        provenance = target + ".provenance.json"
        (root / provenance).write_text(json.dumps(record))
        (root / (target + ".SHA256SUMS")).write_text("".join(f"{sha(root / n)}  {n}\n" for n in (name, provenance)))


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "packages"
        self.root.mkdir()
        fixture(self.root)

    def test_receipts_bind_all_bytes(self):
        self.assertEqual(len(assets(self.root, COMMIT, TAG)[0]), 6)
        next(self.root.glob("*.tar.gz")).write_bytes(b"changed")
        with self.assertRaises(ValueError):
            assets(self.root, COMMIT, TAG)

    def test_extra_and_missing_targets_refused(self):
        extra = self.root / "unexpected.txt"
        extra.write_text("unexpected")
        with self.assertRaises(ValueError):
            assets(self.root, COMMIT, TAG)
        extra.unlink()
        next(self.root.glob("*.provenance.json")).unlink()
        with self.assertRaises(ValueError):
            assets(self.root, COMMIT, TAG)

    def attempt(self, corrupt=False, move_tag=False, fail_at=None, tag=TAG):
        calls, uploaded = [], {}
        def command(*args):
            calls.append(args)
            if args[0] == "api":
                changed = move_tag and len([c for c in calls if c[0] == "api"]) > 1
                return json.dumps({"object": {"type": "commit", "sha": "b" * 40 if changed else COMMIT}})
            if args[1] == fail_at:
                raise subprocess.CalledProcessError(1, ["gh", *args])
            if args[1] == "upload":
                uploaded.update({Path(p).name: Path(p).read_bytes() for p in args[3:args.index("--repo")]})
            if args[1] == "download":
                dest = Path(args[args.index("--dir") + 1]); dest.mkdir()
                for name, value in uploaded.items():
                    (dest / name).write_bytes(b"corrupt" if corrupt else value)
            return ""
        if corrupt or move_tag:
            with self.assertRaises(ValueError):
                publish(self.root, COMMIT, tag, command)
            self.assertFalse(any(c[:2] == ("release", "edit") for c in calls))
        elif fail_at:
            with self.assertRaises(subprocess.CalledProcessError):
                publish(self.root, COMMIT, tag, command)
            self.assertEqual(calls[-1][:2], ("release", fail_at))
            self.assertFalse(any(c[:2] == ("release", "edit") for c in calls))
        else:
            self.assertEqual(publish(self.root, COMMIT, tag, command),
                             f"https://github.com/{REPOSITORY}/releases/tag/{tag}")
            self.assertEqual([c[:2] for c in calls], [
                ("api", f"repos/{REPOSITORY}/git/ref/tags/{tag}"),
                ("release", "create"), ("release", "upload"), ("release", "download"),
                ("api", f"repos/{REPOSITORY}/git/ref/tags/{tag}"), ("release", "edit"),
            ])
            self.assertIn("--draft=false", calls[-1])
            self.assertIn("--latest=false", calls[-1])
        create = calls[1]
        self.assertEqual(create[:3], ("release", "create", tag))
        for flag in ("--draft", "--prerelease", "--verify-tag", "--generate-notes"):
            self.assertIn(flag, create)
        self.assertEqual(create[create.index("--target") + 1], COMMIT)
        self.assertEqual(create[create.index("--repo") + 1], REPOSITORY)
        header = create[create.index("--notes") + 1]
        docs = f"https://github.com/{REPOSITORY}/blob/{tag}/docs"
        self.assertEqual(header, f"[Installation]({docs}/install.md) · "
                                f"[Known limitations]({docs}/limitations.md)\n\n")
        self.assertNotIn("--notes-file", create)
        self.assertFalse(any("--clobber" in c for c in calls))

    def test_verified_download_before_publication(self):
        self.attempt()

    def test_corrupt_download_leaves_draft(self):
        self.attempt(corrupt=True)

    def test_tag_change_leaves_draft(self):
        self.attempt(move_tag=True)

    def test_prerelease_documentation_links_use_requested_tag(self):
        tag = "v2.3.4-rc.1"
        fixture(self.root, tag)
        self.attempt(tag=tag)

    def test_existing_release_is_not_overwritten_or_recovered(self):
        # gh release create refuses an existing release, including a draft.
        self.attempt(fail_at="create")

    def test_upload_or_download_error_leaves_draft(self):
        for operation in ("upload", "download"):
            with self.subTest(operation=operation):
                self.attempt(fail_at=operation)


if __name__ == "__main__":
    unittest.main()
