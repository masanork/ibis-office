#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Check Workers Static Assets free-plan file count and per-file size."""
from pathlib import Path
import sys

root = Path(sys.argv[1] if len(sys.argv) > 1 else 'www')
files = [path for path in root.rglob('*') if path.is_file()]
if not files:
    sys.exit('No static assets found')
limit = 25 * 1024 * 1024
oversized = [path for path in files if path.stat().st_size > limit]
if len(files) > 20_000 or oversized:
    for path in oversized:
        print(f'Asset exceeds 25 MiB: {path} ({path.stat().st_size:,} bytes)', file=sys.stderr)
    sys.exit(f'Workers Static Assets limits exceeded: {len(files):,} files')
largest = max(files, key=lambda path: path.stat().st_size)
print(f'{len(files):,} assets; largest: {largest} ({largest.stat().st_size:,} bytes)')
sw = root / 'sw.js'
if not sw.is_file():
    sys.exit('www/sw.js is missing; run scripts/build-pwa-sw.mjs after the wasm build')
precache = root / 'pwa-precache.json'
if precache.is_file():
    import json
    data = json.loads(precache.read_text())
    urls = data.get('urls') or []
    if './index.html' not in urls or not any('/pkg/' in u or u.startswith('./pkg/') for u in urls):
        sys.exit('pwa-precache.json does not list the app shell and wasm')
    print(f"PWA precache {len(urls)} urls, {data.get('bytes', 0):,} bytes, revision {data.get('revision')}")
