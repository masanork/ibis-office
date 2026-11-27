#!/usr/bin/env python3
"""Verify that a packaged ibisOffice executable includes the gjs dictionary."""

import argparse
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("licenses", type=Path)
    args = parser.parse_args()

    dictionary = (
        Path(__file__).resolve().parents[1]
        / "crates/ibis-doc-model/resources/gjs-variant-clusters.zst"
    ).read_bytes()
    if not dictionary or dictionary not in args.executable.read_bytes():
        raise SystemExit(f"Bundled gjs dictionary missing from {args.executable}")

    credit = "| 異体字クラスターマップ（GJS 基本版） | 異体字照合（ヘルプ > セットアップで完全版を取得可） | Unicode-3.0 AND BSD-3-Clause AND Apache-2.0 AND MIT（調製・提供 楠 正憲） |"
    if credit not in args.licenses.read_text(encoding="utf-8").splitlines():
        raise SystemExit(f"Bundled gjs attribution missing from {args.licenses}")

    print(f"Verified bundled gjs dictionary ({len(dictionary):,} bytes) and attribution")


if __name__ == "__main__":
    main()
