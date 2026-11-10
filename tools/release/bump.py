#!/usr/bin/env python3
"""Write a release's version into the checkout: Cargo.toml, Cargo.lock and the changelog.

Idempotent: a checkout already at the version only gets the changelog section if it
is missing. The workflow commits the result; this script never runs git.
"""
import argparse
from pathlib import Path
import re

from plan import ROOT

CHANGELOG = ROOT / "docs/changelog.md"
HEADER = "# Changelog\n\nChanges per release, newest first. Generated from Conventional Commits.\n"


def set_version(version, root=ROOT):
    old = re.search(r'(?ms)^\[workspace\.package\].*?^version = "([^"]+)"', (root / "Cargo.toml").read_text())[1]
    if old == version:
        return False
    cargo = root / "Cargo.toml"
    text, count = re.subn(r'(?ms)(^\[workspace\.package\].*?^version = ")[^"]+(")', rf"\g<1>{version}\g<2>", cargo.read_text(), count=1)
    if count != 1:
        raise ValueError("Workspace version not found")
    cargo.write_text(text)
    # Workspace crates are the lock entries without a registry source at the old version.
    lock = root / "Cargo.lock"
    def entry(match):
        block = match.group(0)
        if "\nsource = " in block or f'\nversion = "{old}"\n' not in block:
            return block
        return block.replace(f'\nversion = "{old}"\n', f'\nversion = "{version}"\n', 1)
    lock.write_text(re.sub(r'(?ms)^\[\[package\]\]\n.*?(?=^\[\[package\]\]|\Z)', entry, lock.read_text()))
    return True


def add_changelog(version, notes, date, path=CHANGELOG):
    heading = f"## v{version} alpha"
    body = path.read_text() if path.exists() else HEADER
    if not body.startswith("# Changelog"):
        # The first generated release keeps the hand-written entry below it.
        body = HEADER + "\n" + re.sub(r"(?m)^# ", "## ", body, count=1)
    if heading in body:
        return False
    head, rest = body.split("\n## ", 1) if "\n## " in body else (body.rstrip("\n"), "")
    section = f"{heading} ({date})\n\n{notes.strip()}\n"
    path.write_text(head.rstrip("\n") + "\n\n" + section + ("\n## " + rest if rest else ""))
    return True


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--notes", type=Path, required=True)
    parser.add_argument("--date", required=True, help="release date, YYYY-MM-DD")
    args = parser.parse_args()
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", args.date):
        raise SystemExit("Version must be X.Y.Z and date YYYY-MM-DD")
    changed = [set_version(args.version), add_changelog(args.version, args.notes.read_text(), args.date)]
    print("changed" if any(changed) else "unchanged")
