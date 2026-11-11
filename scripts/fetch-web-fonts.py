#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Fetch licensed source fonts independently of GJS; verify before publishing."""
from pathlib import Path
import concurrent.futures
import hashlib
import io
import json
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent


def fetch(spec):
    dest = ROOT / 'data/fonts' / spec['filename']
    if dest.is_file() and hashlib.sha256(dest.read_bytes()).hexdigest() == spec['sha256']:
        return spec['id'] + ': verified existing source'
    request = urllib.request.Request(spec['url'], headers={'User-Agent': 'Mozilla/5.0 ibisOffice-fonts'})
    with urllib.request.urlopen(request, timeout=120) as response:
        data = response.read(128 * 1024 * 1024 + 1)
    if len(data) > 128 * 1024 * 1024:
        raise ValueError('Source archive exceeds download limit: ' + spec['id'])
    if spec['archive']:
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            matches = [entry for entry in archive.infolist() if Path(entry.filename).name == spec['filename']]
            if len(matches) != 1 or matches[0].file_size > 50 * 1024 * 1024:
                raise ValueError('Unexpected font archive: ' + spec['id'])
            data = archive.read(matches[0])
    if len(data) > 50 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != spec['sha256']:
        raise ValueError('Source font SHA-256 mismatch: ' + spec['id'])
    dest.parent.mkdir(parents=True, exist_ok=True)
    temp = dest.with_suffix(dest.suffix + '.download')
    temp.write_bytes(data)
    temp.replace(dest)
    return spec['id'] + ': downloaded and verified'


if __name__ == '__main__':
    catalog = json.loads((ROOT / 'fonts/sources.json').read_text())
    sources = catalog['fonts'] + catalog.get('bootstrapFonts', [])
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        for result in pool.map(fetch, sources):
            print(result, flush=True)
