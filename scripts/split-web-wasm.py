#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Split a wasm-bindgen output into content-addressed static assets."""
import hashlib
import json
import sys
from pathlib import Path

CHUNK_SIZE = 20 * 1024 * 1024


def split_wasm(path: Path) -> None:
    chunks = []
    digest = hashlib.sha256()
    total = 0
    with path.open('rb') as source:
        while data := source.read(CHUNK_SIZE):
            digest.update(data)
            total += len(data)
            sha = hashlib.sha256(data).hexdigest()
            name = f'ibis_office_app_bg.{sha}.chunk'
            (path.parent / name).write_bytes(data)
            chunks.append({'file': name, 'size': len(data), 'sha256': sha})
    if not chunks:
        raise ValueError('Wasm file is empty')
    manifest = {'size': total, 'sha256': digest.hexdigest(), 'chunks': chunks}
    (path.parent / 'ibis_office_app_bg.manifest.json').write_text(
        json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    # The output directory belongs to this build; remove stale chunks and the
    # large original so static asset upload cannot exceed the per-file limit.
    keep = {chunk['file'] for chunk in chunks}
    for old in path.parent.glob('ibis_office_app_bg.*.chunk'):
        if old.name not in keep:
            old.unlink()
    path.unlink()
    path.with_suffix(path.suffix + '.size').unlink(missing_ok=True)
    print(f'Split {total:,} bytes into {len(chunks)} chunks')


if __name__ == '__main__':
    split_wasm(Path(sys.argv[1]))
