#!/usr/bin/env python3
"""Regressions for release planning from Conventional Commits and the version bump."""
from pathlib import Path
import subprocess
import tempfile
import unittest

import bump
import plan


def commit(subject, body=""):
    return plan.classify({"sha": "a" * 40, "subject": subject, "body": body})


def version(current, *subjects):
    return plan.next_version(current, [commit(s) for s in subjects])


class PlanTests(unittest.TestCase):
    def test_fix_perf_and_unformatted_commits_ship_as_patches(self):
        self.assertEqual(version("0.0.16", "fix: x"), "0.0.17")
        self.assertEqual(version("0.0.16", "perf(ui): x"), "0.0.17")
        self.assertEqual(version("0.0.16", "Plain old sentence"), "0.0.17")

    def test_features_are_patches_below_one_and_minors_after(self):
        self.assertEqual(version("0.0.16", "feat: x"), "0.0.17")
        self.assertEqual(version("1.2.3", "fix: a", "feat: b"), "1.3.0")

    def test_breaking_changes_bump_the_leading_part(self):
        self.assertEqual(version("0.0.16", "feat!: x"), "0.1.0")
        self.assertEqual(version("0.3.4", "fix: x", "chore: y"), "0.3.5")
        self.assertEqual(version("1.2.3", "fix(api)!: x"), "2.0.0")
        footer = [commit("fix: x", "Body\n\nBREAKING CHANGE: gone")]
        self.assertEqual(plan.next_version("1.2.3", footer), "2.0.0")

    def test_quiet_types_never_release_alone(self):
        self.assertIsNone(version("0.0.16", "docs: x", "ci: y", "chore(release): v0.0.17", "test: z", "refactor: r"))
        self.assertEqual(version("0.0.16", "docs: x", "fix: y"), "0.0.17")

    def test_notes_group_by_section_and_drop_quiet_commits(self):
        items = [commit("feat(chat): Show receipts"), commit("fix: stop the crash"), commit("docs: words"),
                 commit("feat!: drop it"), commit("Unformatted work")]
        text = plan.notes("0.1.0", items)
        order = [text.index(h) for h in ("### Breaking changes", "### Features", "### Fixes", "### Other changes")]
        self.assertEqual(order, sorted(order))
        self.assertIn("- **chat:** Show receipts (`aaaaaaa`)", text)
        self.assertIn("- Stop the crash", text)
        self.assertNotIn("words", text)
        self.assertIn("blob/v0.1.0/docs/install.md", text)

    def test_subject_format(self):
        for good in ("fix: a", "feat(ui): a", "refactor(core/api)!: a", 'Revert "feat: a"'):
            self.assertTrue(plan.SUBJECT.match(good) or good.startswith("Revert"), good)
        for bad in ("Fix: a", "fix:a", "fix(): a", "update stuff", "wip: a"):
            self.assertFalse(plan.SUBJECT.match(bad) and plan.SUBJECT.match(bad)["type"] in {*plan.RELEASED, *plan.QUIET}, bad)


class RepositoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.saved = plan.ROOT
        plan.ROOT = self.root
        env = ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"]
        def git(*args):
            subprocess.run(["git", *env, *args], cwd=self.root, check=True, capture_output=True)
        self.git = git
        git("init", "-q", "-b", "main")
        for number, subject in enumerate(["initial", "fix: before the tag"]):
            (self.root / "f").write_text(str(number))
            git("add", "f")
            git("commit", "-q", "-m", subject)
        git("tag", "-a", "v0.0.1", "-m", "v0.0.1")
        git("tag", "-a", "v0.0.2-rc.1", "-m", "not stable")

    def tearDown(self):
        plan.ROOT = self.saved
        self.temp.cleanup()

    def add(self, subject, body=""):
        (self.root / "f").write_text(subject)
        self.git("add", "f")
        arguments = ["commit", "-q", "-m", subject]
        if body:
            arguments += ["-m", body]
        self.git(*arguments)

    def test_subject_only_commit_lints_and_plans_without_losing_its_empty_body(self):
        self.add("fix(media): preserve packet boundaries")
        rows = plan.commits("v0.0.1..HEAD")
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["body"], "")
        self.assertEqual(plan.lint("v0.0.1", "HEAD"), 0)
        self.assertEqual(plan.plan()[2], "0.0.2")
        self.assertIn("Preserve packet boundaries", plan.notes("0.0.2", [plan.classify(rows[0])]))

    def test_mixed_bodies_preserve_every_commit_in_a_multi_commit_range(self):
        self.add("fix: oldest without a body")
        self.add("feat: middle", "Details\n\nBREAKING CHANGE: protocol changed")
        self.add("docs: newest without a body")
        rows = plan.commits("v0.0.1..HEAD")
        self.assertEqual([row["subject"] for row in rows], [
            "docs: newest without a body", "feat: middle", "fix: oldest without a body"])
        self.assertEqual((rows[0]["body"], rows[2]["body"]), ("", ""))
        self.assertIn("BREAKING CHANGE: protocol changed", rows[1]["body"])
        self.assertEqual(plan.lint("v0.0.1", "HEAD"), 0)
        self.assertEqual(plan.plan()[2], "0.1.0")

    def test_only_commits_after_the_newest_stable_tag_count(self):
        self.assertEqual(plan.last_tag(), "v0.0.1")
        tag, items, new = plan.plan()
        self.assertEqual((tag, items, new), ("v0.0.1", [], None))
        self.add("docs: words")
        self.assertIsNone(plan.plan()[2])
        self.add("fix(core): repair")
        tag, items, new = plan.plan()
        self.assertEqual((new, len(items)), ("0.0.2", 2))

    def test_lint_flags_only_the_offending_commits(self):
        self.add("feat: fine")
        self.assertEqual(plan.lint("v0.0.1", "HEAD"), 0)
        self.add("Made it nicer")
        self.assertEqual(plan.lint("v0.0.1", "HEAD"), 1)
        self.assertEqual(plan.lint("HEAD~1", "HEAD"), 1)
        self.assertEqual(plan.lint("HEAD~2", "HEAD~1"), 0)


class BumpTests(unittest.TestCase):
    def test_version_lock_and_changelog(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.0.16"\nedition = "2024"\n')
            (root / "Cargo.lock").write_text(
                '[[package]]\nname = "talk-core"\nversion = "0.0.16"\ndependencies = ["x"]\n\n'
                '[[package]]\nname = "outside"\nversion = "0.0.16"\nsource = "registry+https://example.invalid"\n\n'
                '[[package]]\nname = "vendored"\nversion = "0.2.2"\n')
            self.assertTrue(bump.set_version("0.0.17", root))
            self.assertIn('version = "0.0.17"\nedition', (root / "Cargo.toml").read_text())
            lock = (root / "Cargo.lock").read_text()
            self.assertIn('name = "talk-core"\nversion = "0.0.17"', lock)
            self.assertIn('name = "outside"\nversion = "0.0.16"', lock)
            self.assertIn('name = "vendored"\nversion = "0.2.2"', lock)
            self.assertFalse(bump.set_version("0.0.17", root), "idempotent")
            log = root / "changelog.md"
            log.write_text("# v0.0.16 alpha\n\n- old\n")
            self.assertTrue(bump.add_changelog("0.0.17", "### Fixes\n\n- New\n", "2026-11-10", log))
            self.assertFalse(bump.add_changelog("0.0.17", "again", "2026-11-10", log))
            text = log.read_text()
            self.assertTrue(text.startswith("# Changelog"))
            self.assertLess(text.index("## v0.0.17 alpha (2026-11-10)"), text.index("## v0.0.16 alpha"))
            self.assertIn("- old", text)


if __name__ == "__main__":
    unittest.main()
