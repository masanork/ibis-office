#!/usr/bin/env python3
"""Record a CI build's source and hash its staged deliverables, without asserting provenance."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys


def run(*args):
    return subprocess.check_output(args, text=True).rstrip()


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in {"source", "artifacts"}:
        sys.exit("usage: desktop-ci-info.py source|artifacts OUTDIR")
    mode, out = sys.argv[1], Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    if mode == "source":
        head = run("git", "rev-parse", "HEAD")
        expected = os.environ["EXPECTED_SOURCE_SHA"]
        if head != expected:
            sys.exit(f"checkout {head} does not match expected source {expected}")
        status = run("git", "status", "--porcelain", "--untracked-files=no")
        submodules = run("git", "submodule", "status", "--recursive").splitlines()
        if status or any(not line.startswith(" ") for line in submodules):
            sys.exit(f"source or submodules are not clean: {status} {submodules}")
        # Record submodule worktree changes too; a matching gitlink alone does not prove cleanliness.
        dirty = run("git", "submodule", "foreach", "--quiet", "--recursive", "git status --porcelain --untracked-files=no")
        if dirty:
            sys.exit(f"submodule modifications: {dirty}")
        info = {"source_sha": head, "submodules": submodules,
                "rustc": run("rustc", "-Vv"), "cargo": run("cargo", "-V"),
                "runner_os": os.environ.get("RUNNER_OS"), "runner_arch": os.environ.get("RUNNER_ARCH"),
                "run_id": os.environ.get("GITHUB_RUN_ID"), "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
                "repository": os.environ.get("GITHUB_REPOSITORY"),
                "note": "CI build record; not signed provenance."}
        (out / "build-info.json").write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
    else:
        files = sorted(p for p in out.iterdir() if p.is_file() and p.name != "SHA256SUMS")
        if not any(p.name != "build-info.json" for p in files) or not (out / "build-info.json").is_file():
            sys.exit("missing deliverable or build-info.json")
        def checksum(path):
            digest = hashlib.sha256()
            with path.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
            return digest.hexdigest()
        with (out / "SHA256SUMS").open("w", encoding="utf-8", newline="\n") as sums:
            sums.write("".join(f"{checksum(p)}  {p.name}\n" for p in files))


if __name__ == "__main__":
    main()
