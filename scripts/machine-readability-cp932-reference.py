#!/usr/bin/env python3
"""Generate exhaustive 1/2-byte CP932 decoding expectations with Python."""
from pathlib import Path
import struct
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/cp932-python-pairs.bin'
with out.open('wb') as f:
    for size in (1,2):
        for n in range(256**size):
            try:
                decoded=n.to_bytes(size,'big').decode('cp932')
                points=[ord(c) for c in decoded]
                assert 1<=len(points)<=2
                f.write(struct.pack('<II',points[0],points[1] if len(points)==2 else 0x110000))
            except UnicodeDecodeError:
                f.write(struct.pack('<II',0x110001,0x110001))
