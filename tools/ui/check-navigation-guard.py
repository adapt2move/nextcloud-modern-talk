#!/usr/bin/env python3
"""Run the full supplied guard against isolated import mutations (VM only).

No fixture edits the frozen source or canonical checkout. Each case starts with
the same full source archive and changes one import only. This is lexical
boundary evidence, not a resource or Rust correctness proof.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    base = args.build_input.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    cases = [
        ("unchanged-full-guard", None, "", True),
        ("navigation-refcell", "navigation.rs", "use std::cell::RefCell;", True),
        ("navigation-rc", "navigation.rs", "use std::rc::Rc;", True),
        ("navigation-both-grouped", "navigation.rs", "use std::{cell::RefCell, rc::Rc};", True),
        ("other-view-refcell", "settings.rs", "use std::cell::RefCell;", False),
        ("other-view-rc", "settings.rs", "use std::rc::Rc;", False),
        ("navigation-other-cell", "navigation.rs", "use std::cell::Cell;", False),
        ("navigation-forbidden-io", "navigation.rs", "use std::fs::File;", False),
        ("navigation-unreviewed-sync", "navigation.rs", "use std::sync::Arc;", False),
    ]
    results = []
    for name, filename, line, accepted in cases:
        with tempfile.TemporaryDirectory(prefix="ui-nav-guard-") as temp:
            root = Path(temp)
            with tarfile.open(base / "source.tar.gz") as archive:
                archive.extractall(root, filter="data")
            if filename:
                path = root / "apps/desktop/src/presentation" / filename
                path.chmod(0o600)
                path.write_text(path.read_text() + "\n" + line + "\n")
            result = subprocess.run(
                ["python3", "tools/check_architecture.py"], cwd=root,
                capture_output=True, text=True,
            )
            log = args.output / (name + ".log")
            log.write_text(result.stdout + result.stderr)
            passed = (result.returncode == 0) == accepted
            if not accepted:
                passed &= "native presentation import must use core UI" in log.read_text()
            results.append({"case": name, "path": filename, "import": line,
                            "expected_accept": accepted, "exit": result.returncode,
                            "passed": passed, "log": log.name, "log_sha256": sha(log)})
    report = {"source_manifest_sha256": sha(base / "source-manifest.json"),
              "source_archive_sha256": sha(base / "source.tar.gz"),
              "checker_sha256": sha(base / "source/tools/check_architecture.py"),
              "recipe_sha256": sha(Path(__file__)), "results": results}
    (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    raise SystemExit(0 if all(r["passed"] for r in results) else 1)


if __name__ == "__main__":
    main()
