#!/usr/bin/env python3
"""Offline publication failure checks. No GitHub operations."""
import json
from pathlib import Path
import tempfile
import unittest
from publish import assets, publish
from package import sha, TARGETS

COMMIT = "a" * 40
TAG = "v0.0.1"


def fixture(root):
    for target in TARGETS:
        name = target + ".tar.gz"
        (root / name).write_bytes(b"synthetic packaging fixture")
        record = {"kind": "development", "version": TAG[1:], "target": target,
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
        self.notes = self.root.parent / "notes.md"
        self.notes.write_text("Alpha test fixture")

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

    def attempt(self, corrupt=False, move_tag=False):
        calls, uploaded = [], {}
        def command(*args):
            calls.append(args)
            if args[0] == "api":
                changed = move_tag and len([c for c in calls if c[0] == "api"]) > 1
                return json.dumps({"object": {"type": "commit", "sha": "b" * 40 if changed else COMMIT}})
            if args[1] == "upload":
                uploaded.update({Path(p).name: Path(p).read_bytes() for p in args[3:args.index("--repo")]})
            if args[1] == "download":
                dest = Path(args[args.index("--dir") + 1]); dest.mkdir()
                for name, value in uploaded.items():
                    (dest / name).write_bytes(b"corrupt" if corrupt else value)
            return ""
        if corrupt or move_tag:
            with self.assertRaises(ValueError):
                publish(self.root, COMMIT, TAG, self.notes, command)
            self.assertFalse(any(c[:2] == ("release", "edit") for c in calls))
        else:
            publish(self.root, COMMIT, TAG, self.notes, command)
            self.assertEqual(calls[-1][:2], ("release", "edit"))
            self.assertIn("--draft", calls[1])
            self.assertNotIn("--clobber", " ".join(" ".join(c) for c in calls))

    def test_verified_download_before_publication(self):
        self.attempt()

    def test_corrupt_download_leaves_draft(self):
        self.attempt(corrupt=True)

    def test_tag_change_leaves_draft(self):
        self.attempt(move_tag=True)


if __name__ == "__main__":
    unittest.main()
