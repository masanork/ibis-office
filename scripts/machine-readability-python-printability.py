#!/usr/bin/env python3
"""Capture the pinned reference Python's Unicode 15 printability ranges."""
from pathlib import Path
import unicodedata
if unicodedata.unidata_version != '15.0.0': raise SystemExit('reference requires Unicode 15.0.0')
ranges=[];start=None
for n in range(0x110000):
    if not chr(n).isprintable():
        if start is None: start=n
    elif start is not None:
        ranges.append((start,n-1));start=None
if start is not None:ranges.append((start,0x10ffff))
out=Path(__file__).resolve().parent.parent/'crates/ibis-doc-model/resources/harunobu-python-nonprintable.tsv'
out.write_text(''.join(f'{a:X}\t{b:X}\n' for a,b in ranges))
