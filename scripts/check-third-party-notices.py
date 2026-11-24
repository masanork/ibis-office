#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (c) ibis inc.
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.
"""Check the notice copy displayed in the app and the one shipped in its package."""
import argparse
from pathlib import Path
from zipfile import ZipFile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("notice")
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--plex")
    parser.add_argument("--desktop", action="store_true")
    parser.add_argument("--web", action="store_true")
    args = parser.parse_args()
    if args.archive:
        with ZipFile(args.archive) as archive:
            binary = archive.read(args.binary)
            notice = archive.read(args.notice)
            plex = archive.read(args.plex) if args.plex else None
    else:
        binary = Path(args.binary).read_bytes()
        notice = Path(args.notice).read_bytes()
        plex = Path(args.plex).read_bytes() if args.plex else None

    text = notice.decode("utf-8").replace("\r\n", "\n")
    required = ["## 依存元の帰属表示と個別ライセンス"]
    if args.desktop:
        required += [
            "Apache Arrow\nCopyright",
            "Apache Avro\nCopyright",
            "This product includes software developed at The Apache Software Foundation",
            "Code from ttf-parser was used/adapted",
            "Copyright (c) 2017 Just van Rossum",
            " / LICENSE-BoringSSL",
            " / LICENSE-other-bits",
            "Copyright (c) 2009 The Go Authors",
            "Copyright 2015 The Chromium Authors",
            "Copyright 2015-2025 Brian Smith",
        ]
    if args.web:
        required += [
            "| zune-heic |",
            "| zune-isobmff |",
            "Copyright (c) zune-image developers",
        ]
    prose = " ".join(text.split())
    for value in required:
        if " ".join(value.split()) not in prose:
            raise SystemExit(f"Missing notice: {value}")
    if not any(copy in binary for copy in (notice, text.encode("utf-8"))):
        raise SystemExit("The binary and packaged notice differ")

    if plex is not None:
        expected = Path("crates/ibis-office-app/fonts/OFL.txt").read_bytes()
        if plex != expected or expected.decode("utf-8").replace("\r\n", "\n").strip() not in text:
            raise SystemExit("IBM Plex Sans OFL differs from the original")
    print("Verified packaged and in-app third-party notices")


if __name__ == "__main__":
    main()
