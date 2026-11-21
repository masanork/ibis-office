#!/usr/bin/env python3
"""Index the published houritsu paths without checking out the law text."""
from collections import defaultdict
import argparse
import json
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("repository", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()


def git(*command):
    return subprocess.check_output(
        ["git", "-C", str(args.repository), *command], text=True
    )


commit = git("rev-parse", "--verify", "HEAD^{commit}").strip()
paths = {}
for path in git("ls-tree", "-r", "--name-only", commit, "--", "laws").splitlines():
    if match := re.fullmatch(r"laws/[a-z0-9/_-]+/([0-9A-Z]{15})\.md", path):
        paths[match[1]] = path
if not paths:
    raise SystemExit("houritsu の法令ファイルが見つかりません。")
args.output.write_text(
    json.dumps({"commit": commit, "paths": paths}, separators=(",", ":")) + "\n",
    encoding="utf-8",
)

# Match Git commits back to e-Gov revision IDs through amendment metadata.
# Shard by ID prefix so the browser only fetches the relevant part of the index.
refs = ["HEAD"]
if git("for-each-ref", "--format=%(refname)", "refs/heads/upcoming").strip():
    refs.append("refs/heads/upcoming")
changes = defaultdict(dict)
history = git(
    "log", *refs, "--format=%x1e%H%x1f%(trailers:key=Amendment-Law-Id,valueonly)%x1f%(trailers:key=Enforced,valueonly)",
    "--name-only", "--no-renames", "--", "laws",
)
for record in history.split("\x1e")[1:]:
    fields = record.split("\x1f", 2)
    if len(fields) != 3:
        continue
    sha, amendment, body = fields
    amendment = amendment.strip()
    lines = body.splitlines()
    date = lines[0].strip() if lines else ""
    if not re.fullmatch(r"[0-9A-Z]{15}", amendment) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date):
        continue
    for path in lines:
        if match := re.fullmatch(r"laws/[a-z0-9/_-]+/([0-9A-Z]{15})\.md", path):
            law = match[1]
            revision = f"{law}_{date.replace('-', '')}_{amendment}"
            changes[law[:3]].setdefault(revision, sha)

directory = args.output.parent / "history"
directory.mkdir(exist_ok=True)
for old in directory.glob("*.json"):
    old.unlink()
for prefix, shard in changes.items():
    (directory / f"{prefix}.json").write_text(json.dumps(shard, separators=(",", ":")) + "\n", encoding="utf-8")
