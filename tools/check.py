#!/usr/bin/env python3
"""Run the same checks locally and in CI; --full adds tooling and contract suites."""
import argparse
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def run(*argv):
    print("$ " + " ".join(map(str, argv)), flush=True)
    subprocess.run(list(map(str, argv)), cwd=ROOT, check=True)


def python(script, *args):
    run(sys.executable, script, *args)


def cargo(*args):
    python("tools/cargo.py", *args)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--full", action="store_true", help="also run tooling, backend-feature and architecture regression suites")
    args = parser.parse_args(argv)
    try:
        python("tools/check_docs.py")
        python("tools/check_architecture.py")
        python("tools/release/check_workflows.py")
        cargo("fmt", "--all", "--", "--check")
        cargo("clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings")
        cargo("test", "--locked", "--workspace")
        if args.full:
            python("tools/release/check_workflows.py", "--actionlint")
            for script in (
                "tools/check_architecture_tests.py",
                "tools/ci/workflow_policy_tests.py",
                "tools/ci/test_prepare_gstreamer.py",
                "tools/ci/test_macos_signing.py",
                "tools/test_cargo.py",
                "tools/test_dev.py",
                "tools/identity/test_repair_macos_profile.py",
                "tools/release/internal_identity_tests.py",
                "tools/release/tests.py",
                "tools/release/publication_tests.py",
            ):
                python(script)
            cargo("clippy", "--locked", "-p", "talk-adapters", "--features", "backend-test", "--all-targets", "--", "-D", "warnings")
            cargo("test", "--locked", "-p", "talk-adapters", "--features", "backend-test", "--lib")
            with tempfile.TemporaryDirectory(prefix="talk-check-") as temporary:
                probe = Path(temporary) / ("permission-probe.exe" if sys.platform == "win32" else "permission-probe")
                run("rustc", "--edition", "2024", "-D", "warnings", "--test", "tools/native/test-permission-probe.rs", "-o", probe)
                run(probe)
                python("tests/contracts/run_boundary_fixtures.py", "--candidate", ROOT, "--output", Path(temporary) / "boundary.json")
    except subprocess.CalledProcessError as error:
        return error.returncode if error.returncode > 0 else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
