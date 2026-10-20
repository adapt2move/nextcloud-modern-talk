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
        self.check("blacksmith-4vcpu-ubuntu-2404", "ubuntu-latest")

    def test_mutable_action(self):
        self.check("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1", "actions/checkout@main")

    def test_privileged_event(self):
        self.check("pull_request:", "pull_request_target:")

    def test_write_token_in_compile_job(self):
        self.check("    name: Compile only (Windows x86_64 MSVC)", "    permissions:\n      contents: write\n    name: Compile only (Windows x86_64 MSVC)")

    def test_windows_cannot_execute_tests(self):
        self.check("cargo build --locked --workspace --all-targets --target x86_64-pc-windows-msvc", "cargo test --locked --workspace --target x86_64-pc-windows-msvc")

    def test_macos_compile_not_skipped(self):
        self.check("    name: Compile only (macOS Apple Silicon)", "    if: false\n    name: Compile only (macOS Apple Silicon)")


if __name__ == "__main__":
    unittest.main()
