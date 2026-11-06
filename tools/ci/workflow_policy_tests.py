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
        self.check("    name: Compile only (Windows x86_64 MSVC)", "    permissions:\n      contents: write\n    name: Compile only (Windows x86_64 MSVC)")

    def test_windows_cannot_execute_tests(self):
        self.check("python tools/cargo.py build --locked --workspace --all-targets --target x86_64-pc-windows-msvc", "cargo test --locked --workspace --target x86_64-pc-windows-msvc")

    def test_compile_jobs_cannot_execute_through_cargo_wrapper(self):
        for command in ("test", "run", "bench", "nextest"):
            with self.subTest(command=command):
                self.check("python tools/cargo.py build", "python tools/cargo.py " + command)

    def test_wrapped_check_cannot_replace_required_compilation(self):
        self.check("python tools/cargo.py build", "python tools/cargo.py check")

    def test_macos_compile_not_skipped(self):
        self.check("    name: Compile only (macOS Apple Silicon)", "    if: false\n    name: Compile only (macOS Apple Silicon)")

    def test_platform_builds_cannot_run_in_pull_requests(self):
        self.check("    if: " + policy.MAIN_ONLY + "\n", "")
        self.check(policy.MAIN_ONLY, policy.MAIN_ONLY.replace("!=", "=="))

    def test_manual_feature_branches_cannot_run_full_ci(self):
        self.check(policy.MAIN_ONLY, "github.event_name != 'pull_request'")

    def test_linux_must_run_in_pull_requests(self):
        self.check("    name: Linux tests and native smoke",
                   "    if: " + policy.MAIN_ONLY + "\n    name: Linux tests and native smoke")

    def test_linux_can_only_skip_the_explicit_manual_signing_probe(self):
        self.check(policy.LINUX_NORMAL, "false")
        self.check(policy.LINUX_NORMAL, "github.event_name != 'pull_request'")

    def test_signing_probe_cannot_run_automatically_or_use_release_credentials(self):
        self.check(policy.SIGNING_PROBE, "github.event_name == 'pull_request'")
        self.check("    name: Disposable macOS signing contract", "    name: Disposable macOS signing contract\n    env:\n      SECRET: ${{ secrets.MACOS_CERTIFICATE_PASSWORD }}")

    def test_signing_probe_cannot_replace_its_contract_with_app_execution(self):
        self.check("      - run: python3 tools/native/check-macos-signing.py", "      - run: cargo run --locked")
        self.check("      - run: python3 tools/native/check-macos-signing.py", "      - run: python3 tools/native/check-macos-signing.py\n        if: false")

    def test_release_builds_remain_unconditional(self):
        self.check("    runs-on: macos-26", "    if: " + policy.MAIN_ONLY + "\n    runs-on: macos-26",
                   file="release.yml")

    def test_only_unconditional_native_teardown_may_have_a_step_condition(self):
        self.check("        if: always()", "        if: inputs.publish", file="release.yml")


if __name__ == "__main__":
    unittest.main()
