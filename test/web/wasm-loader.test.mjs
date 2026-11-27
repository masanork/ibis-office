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
    const bytes = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
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
    const compileStreaming = WebAssembly.compileStreaming.bind(WebAssembly);
    let streamed = false;
    WebAssembly.compileStreaming = async (response) => {
      const module = await compileStreaming(response);
      streamed = true;
      return module;
    };
    try {
      const loaded = await loadWasm(url, (a, b) => progress.push([a, b]));
      assert.ok(loaded instanceof WebAssembly.Module);
      const expected = await WebAssembly.compile(bytes);
      assert.deepEqual(WebAssembly.Module.imports(loaded), WebAssembly.Module.imports(expected));
      assert.deepEqual(WebAssembly.Module.exports(loaded), WebAssembly.Module.exports(expected));
      assert.equal(streamed, true);
      assert.equal(singleCalls, 1); assert.equal(chunkCalls, 0);
      assert.deepEqual(progress.at(-1), [bytes.length, bytes.length]);
    } finally {
      WebAssembly.compileStreaming = compileStreaming;
    }
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

test('the delivered module omits the Wasm name section and keeps other custom sections', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-wasm-name-'));
  try {
    const withName = Uint8Array.from([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
      0x00, 0x05, 0x04, 0x6b, 0x65, 0x65, 0x70,
      0x00, 0x0a, 0x04, 0x6e, 0x61, 0x6d, 0x65, 0x00, 0x03, 0x02, 0x61, 0x62,
    ]);
    const path = join(dir, 'ibis_office_app_bg.wasm');
    await writeFile(path, withName);
    execFileSync('python3', ['scripts/split-web-wasm.py', path], { stdio: 'pipe' });
    const manifest = JSON.parse(await readFile(join(dir, 'ibis_office_app_bg.manifest.json'), 'utf8'));
    const parts = [];
    for (const chunk of manifest.chunks) parts.push(await readFile(join(dir, chunk.file)));
    const delivered = Buffer.concat(parts);
    assert.equal(manifest.size, 15);
    assert.deepEqual(delivered, Buffer.from(withName.subarray(0, 15)));
    assert.equal(createHash('sha256').update(delivered).digest('hex'), manifest.sha256);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test('compressed split chunks are preferred when single asset exceeds 25 MiB and fall back to raw chunks', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-wasm-br-split-'));
  const originalFetch = globalThis.fetch;
  const originalWarn = console.warn;
  try {
    const chunkA = new Uint8Array(1024);
    const chunkB = new Uint8Array(512);
    chunkA.fill(0x41);
    chunkB.fill(0x42);
    const combined = new Uint8Array(chunkA.length + chunkB.length);
    combined.set(chunkA, 0);
    combined.set(chunkB, chunkA.length);
    const { brotliCompressSync } = await import('node:zlib');
    const brA = brotliCompressSync(chunkA);
    const brB = brotliCompressSync(chunkB);
    const sha = (buf) => createHash('sha256').update(buf).digest('hex');
    await writeFile(join(dir, `ibis_office_app_bg.${sha(chunkA)}.chunk`), chunkA);
    await writeFile(join(dir, `ibis_office_app_bg.${sha(chunkB)}.chunk`), chunkB);
    const manifest = {
      size: combined.length,
      sha256: sha(combined),
      chunks: [
        { file: `ibis_office_app_bg.${sha(chunkA)}.chunk`, size: chunkA.length, sha256: sha(chunkA), br: { sha256: sha(brA), size: brA.length } },
        { file: `ibis_office_app_bg.${sha(chunkB)}.chunk`, size: chunkB.length, sha256: sha(chunkB), br: { sha256: sha(brB), size: brB.length } },
      ],
    };
    let brCalls = 0, rawCalls = 0, failBr = false;
    console.warn = () => {};
    globalThis.fetch = async (url) => {
      const pathname = new URL(url).pathname;
      if (pathname.endsWith('.json')) return Response.json(manifest);
      if (pathname.includes('/pkg/br/')) {
        brCalls++;
        if (failBr) return new Response('', { status: 406 });
        const hash = pathname.split('/').pop().replace('.wasm', '');
        return new Response(hash === sha(brA) ? chunkA : chunkB);
      }
      rawCalls++;
      return new Response(await readFile(join(dir, pathname.split('/').pop())));
    };
    const url = new URL('https://example.test/pkg/ibis_office_app_bg.manifest.json');
    const loaded = await loadWasm(url, () => {});
    assert.deepEqual(new Uint8Array(await loaded.arrayBuffer()), combined);
    assert.equal(brCalls, 2);
    assert.equal(rawCalls, 0);

    failBr = true;
    const fallback = await loadWasm(url, () => {});
    assert.deepEqual(new Uint8Array(await fallback.arrayBuffer()), combined);
    assert.equal(rawCalls, 2);
  } finally {
    globalThis.fetch = originalFetch;
    console.warn = originalWarn;
    await rm(dir, { recursive: true, force: true });
  }
});
