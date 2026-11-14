// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { loadWasm } from '../../www/wasm-loader.js';

test('split assets reconstruct exactly and reject missing, corrupted, or reordered data', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-wasm-'));
  const originalFetch = globalThis.fetch;
  try {
    const original = new Uint8Array(20 * 1024 * 1024 + 17);
    for (let i = 0; i < original.length; i++) original[i] = (i * 13 + (i >>> 16)) & 255;
    const path = join(dir, 'ibis_office_app_bg.wasm');
    await writeFile(path, original);
    execFileSync('python3', ['scripts/split-web-wasm.py', path]);
    const manifest = JSON.parse(await readFile(join(dir, 'ibis_office_app_bg.manifest.json'), 'utf8'));
    assert.equal(manifest.chunks.length, 2);
    assert.ok(manifest.chunks.every(chunk => chunk.size <= 20 * 1024 * 1024));
    let mutation = '';
    globalThis.fetch = async url => {
      const name = new URL(url).pathname.split('/').pop();
      if (name.endsWith('.json')) {
        const data = structuredClone(manifest);
        if (mutation === 'reordered') data.chunks.reverse();
        return Response.json(data);
      }
      if (mutation === 'missing') return new Response('', { status: 404 });
      const bytes = await readFile(join(dir, name));
      if (mutation === 'corrupt') bytes[0] ^= 1;
      return new Response(bytes);
    };
    const progress = [];
    const url = new URL('https://example.test/pkg/ibis_office_app_bg.manifest.json');
    const result = await loadWasm(url, (loaded, total) => progress.push([loaded, total]));
    assert.deepEqual(new Uint8Array(await result.arrayBuffer()), original);
    assert.deepEqual(progress.at(-1), [original.length, original.length]);
    for (mutation of ['missing', 'corrupt', 'reordered']) {
      await assert.rejects(loadWasm(url, () => {}));
    }
  } finally {
    globalThis.fetch = originalFetch;
    await rm(dir, { recursive: true, force: true });
  }
});
