#!/usr/bin/env python3
"""Local release preflight shared by package workflow and direct publisher."""
import os
import re
import subprocess
import sys

REPOSITORY = "adapt2move/nextcloud-modern-talk"
MAINLINE = "main"


def arguments(commit, tag, repository=REPOSITORY):
    if repository != REPOSITORY:
        raise ValueError("Unexpected destination repository")
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[a-z0-9][a-z0-9.-]*)?", tag):
        raise ValueError("Exact source commit and safe version tag required")


def validate_local(commit, tag, repository=REPOSITORY, tagged=True):
    arguments(commit, tag, repository)
    def git(*args):
        return subprocess.check_output(["git", *args], text=True, timeout=30).strip()
    if git("remote", "get-url", "--all", "origin") not in (
        "git@github.com:" + repository + ".git", "https://github.com/" + repository + ".git",
        "https://github.com/" + repository, "ssh://git@github.com/" + repository + ".git",
    ):
        raise ValueError("Local origin is not the intended GitHub repository")
    # Packages are built before the release creates its tag; only publication needs it.
    for ref in ("HEAD", *(("refs/tags/" + tag + "^{commit}",) if tagged else ())):
        if git("rev-parse", ref) != commit:
            raise ValueError("Checkout/tag mismatch")
    subprocess.run(["git", "merge-base", "--is-ancestor", commit, "refs/remotes/origin/" + MAINLINE], check=True, timeout=30, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if git("status", "--porcelain"):
        raise ValueError("Dirty release candidate")


if __name__ == "__main__":
    validate_local(os.environ["CANDIDATE"], os.environ["VERSION_TAG"], tagged="--untagged" not in sys.argv[1:])
    print("Clean local origin/tag/mainline preflight passed; remote publication policy and acceptance are separate.")
