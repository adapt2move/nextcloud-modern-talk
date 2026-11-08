#!/usr/bin/env python3
"""Small regressions for the repository's runner and token policy."""
import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("workflow_policy", ROOT / "tools/release/check_workflows.py")
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class PolicyTests(unittest.TestCase):
    def check(self, old=None, new=None, file="ci.yml"):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            shutil.copytree(ROOT / ".github", root / ".github")
            (root / "tools/release").mkdir(parents=True)
            shutil.copy(ROOT / "tools/release/actions.json", root / "tools/release/actions.json")
            if old is not None:
                path = root / ".github/workflows" / file
                body = path.read_text()
                self.assertIn(old, body)
                path.write_text(body.replace(old, new, 1))
                with self.assertRaises(ValueError):
                    policy.check(root)
            else:
                policy.check(root)

    def test_current_workflows(self):
        self.check()

    def test_unselected_runner(self):
        self.check("ubuntu-26.04", "ubuntu-latest")

    def test_mutable_action(self):
        self.check("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1", "actions/checkout@main")

    def test_privileged_event(self):
        self.check("pull_request:", "pull_request_target:")

    def test_write_token_in_compile_job(self):
        self.check("    name: Compile only (Windows x86_64 MSVC)", "    permissions:\n      contents: write\n    name: Compile only (Windows x86_64 MSVC)", file="native.yml")

    def test_windows_cannot_execute_tests(self):
        self.check("python tools/cargo.py build --locked --workspace --all-targets --target x86_64-pc-windows-msvc", "cargo test --locked --workspace --target x86_64-pc-windows-msvc", file="native.yml")

    def test_compile_jobs_cannot_execute_through_cargo_wrapper(self):
        for command in ("test", "run", "bench", "nextest"):
            with self.subTest(command=command):
                self.check("python tools/cargo.py build", "python tools/cargo.py " + command, file="native.yml")

    def test_wrapped_check_cannot_replace_required_compilation(self):
        self.check("python tools/cargo.py build", "python tools/cargo.py check", file="native.yml")

    def test_macos_compile_not_skipped(self):
        self.check("    name: Compile and package only (macOS Apple Silicon)", "    if: false\n    name: Compile and package only (macOS Apple Silicon)", file="native.yml")

    def test_native_checks_are_manual_and_credential_free(self):
        self.check("  workflow_dispatch:", "  pull_request:", file="native.yml")
        self.check("  workflow_dispatch:", "  workflow_dispatch:\n  push:", file="native.yml")
        self.check("    name: Compile only (Windows x86_64 MSVC)", "    name: Compile only (Windows x86_64 MSVC)\n    env:\n      SECRET: ${{ secrets.MACOS_CERTIFICATE_PASSWORD }}", file="native.yml")

    def test_linux_check_cannot_be_skipped_or_replaced(self):
        self.check("    name: Checks", "    if: false\n    name: Checks")
        self.check("python3 tools/check.py", "true")

    def test_release_builds_remain_unconditional(self):
        self.check("    runs-on: macos-26", "    if: false\n    runs-on: macos-26",
                   file="release.yml")

    def test_publication_requires_publisher_and_no_local_notes_file(self):
        self.check('"$VERSION_TAG" --publish', '"$VERSION_TAG" --notes notes.md --publish',
                   file="release.yml")
        self.check('"$VERSION_TAG" --publish', '"$VERSION_TAG"', file="release.yml")
        self.check("tools/release/publish.py", "other-publisher.py", file="release.yml")

    def test_only_unconditional_native_teardown_may_have_a_step_condition(self):
        self.check("        if: always()", "        if: inputs.publish", file="release.yml")


if __name__ == "__main__":
    unittest.main()
