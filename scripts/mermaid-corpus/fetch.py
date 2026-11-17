#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Refresh the Mermaid compatibility corpus from the official mermaid docs.

Downloads `docs/syntax/*.md` from mermaid-js/mermaid at a pinned commit and
writes every distinct ```mermaid / ```mermaid-example block to
`crates/ibis-pptx/tests/mermaid/corpus/<doc>.mmd`, separated by
`%%%% case <n> (<doc>.md:<line>)` lines. The examples are MIT licensed
(Copyright (c) 2014 - 2022 Knut Sveidqvist); see the corpus LICENSE file.

Usage: scripts/mermaid-corpus/fetch.py [--ref <tag-or-sha>]
"""
import argparse
import json
import pathlib
import re
import urllib.request

DEFAULT_TAG = "mermaid@12.3.0"
DEFAULT_SHA = "2086904e76c692331e34a45d7cc3bc30860d040e"
OUT = pathlib.Path(__file__).resolve().parents[2] / "crates/ibis-pptx/tests/mermaid/corpus"
FENCE = re.compile(r"^```(mermaid|mermaid-example)\s*$")


def get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "ibis-office-mermaid-corpus"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def extract(text: str):
    lines = text.split("\n")
    i, out = 0, []
    while i < len(lines):
        if FENCE.match(lines[i].strip()):
            start = i + 1
            j = start
            while j < len(lines) and lines[j].strip() != "```":
                j += 1
            body = "\n".join(lines[start:j]).strip("\n")
            if body.strip():
                out.append((start, body))
            i = j + 1
        else:
            i += 1
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref", default=DEFAULT_SHA)
    ap.add_argument("--tag", default=DEFAULT_TAG)
    args = ap.parse_args()
    listing = json.loads(get(f"https://api.github.com/repos/mermaid-js/mermaid/contents/docs/syntax?ref={args.ref}"))
    names = sorted(e["name"] for e in listing if e["name"].endswith(".md") and e["name"] != "all-examples.md")
    OUT.mkdir(parents=True, exist_ok=True)
    for old in OUT.glob("*.mmd"):
        old.unlink()
    manifest = {"source": "https://github.com/mermaid-js/mermaid", "tag": args.tag, "commit": args.ref, "docs": {}}
    for name in names:
        text = get(f"https://raw.githubusercontent.com/mermaid-js/mermaid/{args.ref}/docs/syntax/{name}").decode("utf-8")
        seen, cases = set(), []
        for line, body in extract(text):
            if body in seen:
                continue
            seen.add(body)
            cases.append((line, body))
        if not cases:
            continue
        stem = name[:-3]
        with open(OUT / f"{stem}.mmd", "w", encoding="utf-8", newline="\n") as f:
            for n, (line, body) in enumerate(cases, 1):
                f.write(f"%%%% case {n} ({name}:{line})\n{body}\n")
        manifest["docs"][stem] = len(cases)
    (OUT / "SOURCE.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest["docs"], indent=1))


if __name__ == "__main__":
    main()
