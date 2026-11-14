// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { brotliDecompressSync } from 'node:zlib';
import { createHash } from 'node:crypto';
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
    const encoded = await readFile(join(dir, `wasm-br.${manifest.single.sha256}.bin`));
    assert.equal(encoded.length, manifest.single.size);
    assert.equal(createHash('sha256').update(encoded).digest('hex'), manifest.single.sha256);
    assert.deepEqual(new Uint8Array(brotliDecompressSync(encoded)), original);
    delete manifest.single; // Exercise legacy manifests and the split path independently.
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


test('single decoded Wasm is verified, with fallback for unsupported, corrupt or truncated responses', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-wasm-single-'));
  const originalFetch = globalThis.fetch;
  const originalWarn = console.warn;
  try {
    const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 10, 20, 30]);
    const path = join(dir, 'ibis_office_app_bg.wasm');
    await writeFile(path, bytes);
    execFileSync('python3', ['scripts/split-web-wasm.py', path]);
    const manifest = JSON.parse(await readFile(join(dir, 'ibis_office_app_bg.manifest.json'), 'utf8'));
    let failure = '', singleCalls = 0, chunkCalls = 0;
    console.warn = () => {};
    globalThis.fetch = async url => {
      const name = new URL(url).pathname.split('/').pop();
      if (name.endsWith('.json')) return Response.json(manifest);
      if (name.endsWith('.wasm')) {
        singleCalls++;
        if (failure === 'unsupported') return new Response('', { status: 406 });
        if (failure === 'network') throw new TypeError('network error');
        if (failure === 'corrupt') return new Response(Uint8Array.from(bytes, (b, i) => i === 0 ? b ^ 1 : b));
        if (failure === 'truncated') return new Response(bytes.slice(0, -1));
        if (failure === 'oversized') return new Response(new Uint8Array(bytes.length + 1));
        return new Response(bytes);
      }
      chunkCalls++;
      return new Response(await readFile(join(dir, name)));
    };
    const url = new URL('https://example.test/pkg/ibis_office_app_bg.manifest.json');
    const progress = [];
    assert.deepEqual(new Uint8Array(await (await loadWasm(url, (a, b) => progress.push([a, b]))).arrayBuffer()), bytes);
    assert.equal(singleCalls, 1); assert.equal(chunkCalls, 0);
    assert.deepEqual(progress.at(-1), [bytes.length, bytes.length]);
    for (failure of ['unsupported', 'network', 'corrupt', 'truncated', 'oversized']) {
      const before = chunkCalls;
      assert.deepEqual(new Uint8Array(await (await loadWasm(url, () => {})).arrayBuffer()), bytes, failure);
      assert.ok(chunkCalls > before, `split fallback was not used: ${failure}`);
    }
    const before = singleCalls;
    manifest.single.sha256 = 'invalid';
    assert.deepEqual(new Uint8Array(await (await loadWasm(url, () => {})).arrayBuffer()), bytes);
    assert.equal(singleCalls, before, 'invalid optional metadata must use the split path');
  } finally {
    globalThis.fetch = originalFetch;
    console.warn = originalWarn;
    await rm(dir, { recursive: true, force: true });
  }
});
