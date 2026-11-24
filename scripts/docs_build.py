# SPDX-License-Identifier: AGPL-3.0-only
"""Identify a Docs build from its checked-out Git history."""

import subprocess
import sys
from pathlib import Path


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def build_number():
    if git("rev-parse", "--is-shallow-repository") == "true":
        return None
    return int(git("rev-list", "--first-parent", "--count", "HEAD"))


def version_for(platform):
    versions = dict(line.split("\t", 1) for line in Path("version.txt").read_text().splitlines() if line)
    return versions[platform].strip()


def describe(platform):
    number = build_number()
    if number is None:
        return {}
    return {
        "build_number": number,
        "build_name": f"docs-v{version_for(platform)}-build{number}",
    }


def prepare_history():
    if git("rev-parse", "--is-shallow-repository") == "true":
        source = git("rev-parse", "HEAD")
        subprocess.run(
            ["git", "fetch", "--unshallow", "--no-tags", "--no-recurse-submodules", "origin", source],
            check=True,
        )
    if build_number() is None:
        raise ValueError("complete Git history is required for the Docs build number")


if __name__ == "__main__":
    if sys.argv[1:] == ["prepare-history"]:
        prepare_history()
    else:
        sys.exit("usage: docs_build.py prepare-history")
