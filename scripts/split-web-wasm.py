#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Split a wasm-bindgen output into content-addressed static assets."""
import hashlib
import json
import sys
import subprocess
import tempfile
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
    # Keep a content-addressed single Brotli asset when it fits the static
    # host's 25 MiB limit. The chunks remain available for other hosts and
    # clients that cannot use this representation.
    temp = tempfile.NamedTemporaryFile(prefix='wasm-brotli-', suffix='.tmp', dir=path.parent, delete=False)
    temp_path = Path(temp.name)
    temp.close()
    temp_path.unlink()
    try:
        subprocess.run(['node', str(Path(__file__).with_name('compress-web-wasm.mjs')), str(path), str(temp_path)], check=True)
        compressed = temp_path.read_bytes()
        manifest = {'size': total, 'sha256': digest.hexdigest(), 'chunks': chunks}
        if len(compressed) <= 25 * 1024 * 1024:
            encoded_hash = hashlib.sha256(compressed).hexdigest()
            encoded_name = f'wasm-br.{encoded_hash}.bin'
            temp_path.replace(path.parent / encoded_name)
            manifest['single'] = {'sha256': encoded_hash, 'size': len(compressed)}
        else:
            print(f'Brotli asset is {len(compressed):,} bytes; retaining split-only delivery')
    finally:
        temp_path.unlink(missing_ok=True)
    (path.parent / 'ibis_office_app_bg.manifest.json').write_text(
        json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    # The output directory belongs to this build; remove stale chunks and the
    # large original so static asset upload cannot exceed the per-file limit.
    keep = {chunk['file'] for chunk in chunks}
    for old in path.parent.glob('wasm-br.*.bin'):
        if old.name != (f"wasm-br.{manifest['single']['sha256']}.bin" if 'single' in manifest else ''):
            old.unlink()
    for old in path.parent.glob('ibis_office_app_bg.*.chunk'):
        if old.name not in keep:
            old.unlink()
    path.unlink()
    path.with_suffix(path.suffix + '.size').unlink(missing_ok=True)
    print(f'Split {total:,} bytes into {len(chunks)} chunks')


if __name__ == '__main__':
    split_wasm(Path(sys.argv[1]))
