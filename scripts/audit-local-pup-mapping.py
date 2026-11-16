#!/usr/bin/env python3
"""Cross-check a local PUP/IVS CSV against MJ-PUP and MJ character data.

No source data or derived mapping is distributed by this script. The two
reference files can themselves be related to the data being checked, so a
match is an integrity check, not independent proof of provenance.
"""
import argparse
import csv
import sys
from collections import Counter
from pathlib import Path


def dictionaries(path):
    with path.open(encoding="utf-8-sig", newline="") as stream:
        yield from csv.DictReader(stream)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mapping", type=Path)
    parser.add_argument("mj_pup", type=Path)
    parser.add_argument("mj_info", type=Path)
    args = parser.parse_args()
    pups = {}
    for row in dictionaries(args.mj_pup):
        code = row["後方互換PUPコード"]
        if not code.startswith("U+"):
            continue
        point = int(code[2:], 16)
        mj = row["MJ文字図形名"]
        if point in pups and pups[point] != mj:
            raise ValueError(f"duplicate PUP U+{point:X}: {pups[point]} / {mj}")
        pups[point] = mj
    characters = {}
    for row in dictionaries(args.mj_info):
        mj = row["MJ文字図形名"]
        if mj in characters:
            raise ValueError(f"duplicate MJ: {mj}")
        characters[mj] = row["実装したMoji_JohoコレクションIVS"].upper()

    counts = Counter()
    seen = set()
    with args.mapping.open(encoding="utf-8-sig", newline="") as stream:
        for row in csv.reader(stream):
            if len(row) != 2 or len(row[0]) != 1 or len(row[1]) < 2:
                counts["malformed"] += 1
                continue
            point = ord(row[0])
            if point in seen:
                counts["duplicate"] += 1
                continue
            seen.add(point)
            mj = pups.get(point)
            expected = characters.get(mj)
            if expected is None:
                counts["missing_reference"] += 1
                continue
            actual = "_".join(f"{ord(ch):X}" for ch in row[1][:2])
            counts["match" if actual == expected else "mismatch"] += 1
    print(f"rows={sum(counts.values())} " + " ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    return int(not seen or any(counts[k] for k in ("malformed", "duplicate", "missing_reference", "mismatch")))


if __name__ == "__main__":
    sys.exit(main())
