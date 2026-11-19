#!/usr/bin/env python3
"""Build a bitmap dictionary from a reviewed CSV or pinned Unicode 15.0 data."""
import argparse
import csv
import hashlib
import pathlib
import struct
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('--unicode-data', action='store_true', help='use Unicode 15.0.0 UnicodeData.txt instead of CSV')
parser.add_argument('source', type=pathlib.Path)
parser.add_argument('output', type=pathlib.Path, help='output .zst file')
args = parser.parse_args()
source = args.source.read_bytes()
digest = hashlib.sha256(source).hexdigest()
if args.unicode_data:
    if digest != '806e9aed65037197f1ec85e12be6e8cd870fc5608b4de0fffd990f689f376a73':
        raise ValueError('expected the pinned Unicode 15.0.0 UnicodeData.txt')
    representatives = {}
    for line in source.decode('utf-8').splitlines():
        fields = line.split(';')
        if fields[1].startswith('CJK COMPATIBILITY IDEOGRAPH-') and fields[5]:
            member, representative = int(fields[0], 16), int(fields[5], 16)
            representatives[member] = representative
            representatives.setdefault(representative, representative)
    rows = [
        dict(member_codepoint=hex(member), cluster_id=hex(rep), representative_codepoint=hex(rep))
        for member, rep in sorted(representatives.items())
    ]
else:
    rows = list(csv.DictReader(source.decode('utf-8-sig').splitlines()))
groups = {}
reps = []
pairs = []
for row in rows:
    cp = int(row['member_codepoint'], 16)
    rep = int(row['representative_codepoint'], 16)
    key = row['cluster_id']
    if not (0 <= cp <= 0x10ffff and not 0xd800 <= cp <= 0xdfff and 0 <= rep <= 0x10ffff and not 0xd800 <= rep <= 0xdfff):
        raise ValueError('invalid Unicode scalar')
    if key not in groups:
        groups[key] = len(groups)
        reps.append(rep)
    g = groups[key]
    if reps[g] != rep:
        raise ValueError('inconsistent representative')
    pairs.append((cp, g))
if not pairs or len(groups) > 65535 or any(a[0] >= b[0] for a, b in zip(pairs, pairs[1:])):
    raise ValueError('empty input, too many groups, or members not strictly sorted')
members = [[] for _ in groups]
for cp, group in pairs:
    members[group].append(cp)
for group, rep in enumerate(reps):
    if rep not in members[group]:
        raise ValueError('representative missing from cluster')
p16 = lambda values: b''.join(struct.pack('<H', x) for x in values)
p32 = lambda values: b''.join(struct.pack('<I', x) for x in values)
offsets = [0]
for m in members:
    offsets.append(offsets[-1] + len(m))
pages = sorted({cp >> 8 for cp, _ in pairs})
page_indices = {page: i for i, page in enumerate(pages)}
directory = [0] * 4352
for page, i in page_indices.items():
    directory[page] = i + 1
bitmaps = [[0] * 8 for _ in pages]
for cp, _ in pairs:
    bitmaps[page_indices[cp >> 8]][(cp & 255) // 32] |= 1 << (cp % 32)
rank = 0
bitmap = b''
for words in bitmaps:
    bitmap += struct.pack('<I', rank) + p32(words)
    rank += sum(bin(w).count("1") for w in words)
assert rank == len(pairs)
raw = (struct.pack('<IIII', len(pairs), len(groups), len(pages), 0) + p16(directory) + bitmap
       + p16(g for _, g in pairs) + p32(offsets) + p32(reps) + p32(cp for group in members for cp in group))
with tempfile.TemporaryDirectory() as tmp:
    path = pathlib.Path(tmp) / 'dictionary.bin'
    path.write_bytes(raw)
    subprocess.run(['zstd', '-q', '-19', '-f', str(path), '-o', str(args.output)], check=True)
print(f'Source sha256={digest} members={len(pairs)} clusters={len(groups)} pages={len(pages)} raw={len(raw)} zstd={args.output.stat().st_size}')
