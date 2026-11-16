#!/usr/bin/env python3
"""Build a local bitmap dictionary from a reviewed member/cluster/representative CSV."""
import argparse
import csv
import hashlib
import pathlib
import struct
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('csv', type=pathlib.Path)
parser.add_argument('output', type=pathlib.Path, help='output .zst file')
args = parser.parse_args()
source = args.csv.read_bytes()
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
print(f'CSV sha256={hashlib.sha256(source).hexdigest()} members={len(pairs)} clusters={len(groups)} pages={len(pages)} raw={len(raw)} zstd={args.output.stat().st_size}')
