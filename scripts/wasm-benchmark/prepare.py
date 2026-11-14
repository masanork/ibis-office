#!/usr/bin/env python3
"""Prepare a public, isolated Wasm benchmark from an existing split build."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('manifest', type=Path, help='path to ibis_office_app_bg.manifest.json')
parser.add_argument('output', type=Path, help='new directory for generated assets')
args = parser.parse_args()
manifest = json.loads(args.manifest.read_text())
if args.output.exists():
    raise SystemExit('output must be a new directory')
args.output.mkdir(parents=True)
whole = bytearray()
for index, entry in enumerate(manifest['chunks']):
    name = entry['file']
    if Path(name).name != name:
        raise SystemExit('invalid chunk name')
    data = (args.manifest.parent / name).read_bytes()
    if len(data) != entry['size'] or hashlib.sha256(data).hexdigest() != entry['sha256']:
        raise SystemExit(f'invalid chunk: {name}')
    whole.extend(data)
    (args.output / f'part{index}.bin').write_bytes(gzip.compress(data, compresslevel=6, mtime=0))
if len(whole) != manifest['size'] or hashlib.sha256(whole).hexdigest() != manifest['sha256']:
    raise SystemExit('invalid reconstructed Wasm')
if len(manifest['chunks']) != 4:
    raise SystemExit('this comparison fixture expects four chunks')
(args.output / 'single.bin').write_bytes(gzip.compress(whole, compresslevel=6, mtime=0))
with tempfile.NamedTemporaryFile(prefix='wasm-source-', dir=args.output) as raw:
    raw.write(whole)
    raw.flush()
    subprocess.run(['brotli', '-q', '5', raw.name, '-o', str(args.output / 'single-br.bin')], check=True)
(args.output / 'manifest.json').write_text(json.dumps(manifest) + '\n')
(args.output / 'index.html').write_text('<!doctype html><title>Wasm delivery comparison</title>')
for asset in sorted(args.output.iterdir()):
    if asset.stat().st_size > 25 * 1024 * 1024:
        raise SystemExit(f'asset exceeds 25 MiB: {asset.name}')
    print(asset.name, asset.stat().st_size)
