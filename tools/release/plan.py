#!/usr/bin/env python3
"""Plan a release from Conventional Commits; check new commits against the format.

`plan` reads the commits since the newest `vX.Y.Z` tag, picks the next version and
writes the release notes. `lint` rejects commits that would not classify. Nothing
here talks to GitHub or changes the checkout.

Bump rules (documented in CONTRIBUTING.md):
  - `!` after the type or a `BREAKING CHANGE:` footer: major, or minor below 1.0.
  - `feat`: minor from 1.0 on; patch below 1.0 (the alpha ships features as patches).
  - `fix`, `perf`, `revert`, and any commit that is not in the format: patch.
  - `build`, `chore`, `ci`, `docs`, `refactor`, `style`, `test`: no release alone.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

REPOSITORY = "adapt2move/nextcloud-modern-talk"
ROOT = Path(__file__).resolve().parents[2]
RELEASED = {"feat": "Features", "fix": "Fixes", "perf": "Performance", "revert": "Reverts"}
QUIET = {"build", "chore", "ci", "docs", "refactor", "style", "test"}
SECTIONS = ["Breaking changes", "Features", "Fixes", "Performance", "Reverts", "Other changes"]
SUBJECT = re.compile(
    r"^(?P<type>[a-z]+)(?:\((?P<scope>[a-z0-9][a-z0-9._/-]*)\))?(?P<bang>!)?: (?P<text>\S.*)$"
)
BREAKING = re.compile(r"(?m)^BREAKING[ -]CHANGE: ")
STABLE_TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
MAX_SUBJECT = 100


def git(*args):
    # strip() also consumes the control-character field/record separators used
    # by commits(), losing the final empty body of a subject-only commit.
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True, timeout=60).rstrip("\n")


def workspace_version():
    match = re.search(r'(?ms)^\[workspace\.package\].*?^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text())
    if not match:
        raise ValueError("No workspace version in Cargo.toml")
    return match[1]


def last_tag():
    tags = [t for t in git("tag", "--merged", "HEAD").splitlines() if STABLE_TAG.match(t)]
    if not tags:
        raise ValueError("No vX.Y.Z tag to count commits from")
    return max(tags, key=lambda t: tuple(map(int, STABLE_TAG.match(t).groups())))


def commits(revision_range):
    raw = git("log", "--no-merges", "--format=%H%x1f%s%x1f%b%x1e", revision_range)
    rows = []
    for entry in raw.split("\x1e"):
        entry = entry.strip("\n")
        if entry:
            sha, subject, body = entry.split("\x1f", 2)
            rows.append({"sha": sha, "subject": subject.strip(), "body": body})
    return rows


def classify(commit):
    """Type, scope, text, breaking flag and whether it counts for a release."""
    subject, body = commit["subject"], commit["body"]
    match = SUBJECT.match(subject)
    if subject.startswith('Revert "'):
        kind, scope, text, bang = "revert", None, subject, False
    elif match and (match["type"] in RELEASED or match["type"] in QUIET):
        kind, scope, text, bang = match["type"], match["scope"], match["text"], bool(match["bang"])
    else:
        # Work that predates the format, or a mistake `lint` catches on new commits.
        kind, scope, text, bang = "other", None, subject, False
    breaking = bang or bool(BREAKING.search(body))
    released = kind in RELEASED or kind == "other" or breaking
    section = "Breaking changes" if breaking else RELEASED.get(kind, "Other changes")
    return {**commit, "type": kind, "scope": scope, "text": text, "breaking": breaking,
            "released": released, "section": section}


def next_version(current, items):
    major, minor, patch = map(int, current.split("."))
    released = [i for i in items if i["released"]]
    if not released:
        return None
    if any(i["breaking"] for i in released):
        return f"{major + 1}.0.0" if major else f"0.{minor + 1}.0"
    if major and any(i["type"] == "feat" for i in released):
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def notes(version, items, links=True):
    lines = []
    for section in SECTIONS:
        entries = [i for i in items if i["released"] and i["section"] == section]
        if not entries:
            continue
        lines += [f"### {section}", ""]
        for item in entries:
            scope = f"**{item['scope']}:** " if item["scope"] else ""
            text = item["text"][0].upper() + item["text"][1:]
            lines.append(f"- {scope}{text} (`{item['sha'][:7]}`)")
        lines.append("")
    if links:
        base = f"https://github.com/{REPOSITORY}/blob/v{version}/docs"
        lines.append(f"[Installation]({base}/install.md) · [Known limitations]({base}/limitations.md)")
    return "\n".join(lines).rstrip("\n") + "\n"


def plan():
    tag = last_tag()
    items = [classify(c) for c in commits(f"{tag}..HEAD")]
    current = tag[1:]
    version = next_version(current, items)
    return tag, items, version


def write_plan(output, github_output):
    tag, items, version = plan()
    if version is None:
        skipped = ", ".join(sorted({i["type"] for i in items})) or "none"
        raise SystemExit(f"Nothing to release since {tag}: only {skipped} commits (no feat, fix, perf or breaking change).")
    head = git("rev-parse", "HEAD")
    body = notes(version, items)
    result = {"previous": tag, "version": version, "tag": f"v{version}", "head": head,
              "commits": sum(i["released"] for i in items)}
    if output:
        output = Path(output)
        output.mkdir(parents=True, exist_ok=True)
        (output / "notes.md").write_text(body)
        (output / "plan.json").write_text(json.dumps(result, indent=2) + "\n")
    if github_output:
        with open(github_output, "a") as handle:
            for key in ("version", "tag", "head"):
                handle.write(f"{key}={result[key]}\n")
    print(json.dumps(result, indent=2))
    print()
    print(body)


def lint(start, end):
    bad = []
    for commit in commits(f"{start}..{end}"):
        subject = commit["subject"]
        match = SUBJECT.match(subject)
        valid = subject.startswith('Revert "') or (
            match and (match["type"] in RELEASED or match["type"] in QUIET) and len(subject) <= MAX_SUBJECT)
        if not valid:
            bad.append(f"{commit['sha'][:7]} {subject}")
    if bad:
        types = ", ".join(sorted({*RELEASED, *QUIET}))
        print("Commit subjects must be `type(scope)!: description` "
              f"(types: {types}; at most {MAX_SUBJECT} characters). Offending commits:")
        print("\n".join("  " + line for line in bad))
        return 1
    print("Commit subjects follow the Conventional Commits format.")
    return 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("plan", help="print the next version and release notes")
    p.add_argument("--output", type=Path, help="directory for notes.md and plan.json")
    p.add_argument("--github-output", default=os.environ.get("GITHUB_OUTPUT"))
    p = sub.add_parser("lint", help="check commit subjects in start..end")
    p.add_argument("start")
    p.add_argument("end", nargs="?", default="HEAD")
    sub.add_parser("version", help="print the version in Cargo.toml")
    args = parser.parse_args()
    if args.command == "plan":
        write_plan(args.output, args.github_output)
    elif args.command == "lint":
        sys.exit(lint(args.start, args.end))
    else:
        print(workspace_version())
