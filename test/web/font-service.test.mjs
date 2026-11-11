// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createFontService } from '../../worker/font-service.js';
import { loadSubsetter } from '../../scripts/font-subsetter.mjs';
import { namespaceSubset } from '../../www/dynamic-fonts.js';
import { checksum, fontFamily, tables } from '../../scripts/web-font-sfnt.mjs';

const bg = await loadSubsetter();
const bytes = new Uint8Array(await readFile(new URL('../../crates/ibis-font-subsetter/tests/fonts/ivs.ttf', import.meta.url)));
const fonts = [{ id: 'test', key: 'source.ttf', license: 'AGPL-3.0-only', originalUrl: 'https://example.test/source' }];
const request = (text = 'AA\u{E0100}m', extra = '') => new Request('https://ibis.test/api/font/subset?font=test&text=' + encodeURIComponent(text) + extra);

function setup() {
  const entries = new Map();
  const pending = [];
  let reads = 0, frees = 0, etag = 'v1', allowed = true, source = bytes, size = bytes.length;
  let canceled = false;
  const context = {
    store: {
      async head() { return { size, etag }; },
      async read(_key, expected) {
        assert.equal(expected, etag);
        reads++;
        let offset = 0;
        return new ReadableStream({
          pull(controller) {
            if (offset >= source.length) return controller.close();
            controller.enqueue(source.slice(offset, offset += 127));
          },
          cancel() { canceled = true; },
        });
      },
    },
    cache: {
      async match(key) { return entries.get(key.url)?.clone(); },
      async put(key, value) { entries.set(key.url, new Response(await value.arrayBuffer(), value)); },
    },
    async allowed() { return allowed; },
    waitUntil(work) { pending.push(work); },
  };
  const factory = size => {
    const engine = new bg.FontSubsetter(size);
    return {
      append: chunk => engine.append(chunk),
      has_glyphs: text => engine.has_glyphs(text),
      subset: (text, woff2) => engine.subset(text, woff2),
      free() { frees++; engine.free(); },
    };
  };
  return { context, factory, serve: createFontService(fonts, 'engine1', factory), pending,
    get reads() { return reads; }, get frees() { return frees; }, get canceled() { return canceled; },
    revision(value) { etag = value; }, rate(value) { allowed = value; },
    stream(value, declared = bytes.length) { source = value; size = declared; },
  };
}

test('own WASM service streams IVS font, retains attribution and caches only exact source/version/format/text', async () => {
  const s = setup();
  const response = await s.serve(request(), s.context);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('x-font-id'), 'test');
  assert.equal(response.headers.get('x-font-license'), 'AGPL-3.0-only');
  assert.equal(response.headers.get('x-original-font'), fonts[0].originalUrl);
  assert.equal(response.headers.get('x-cache'), 'MISS');
  const result = new Uint8Array(await response.arrayBuffer());
  assert.equal(fontFamily(result), 'IBS Subset');
  const namespaced = namespaceSubset(result, 1);
  assert.equal(fontFamily(namespaced), 'IBS 000001');
  assert.equal(checksum(namespaced), 0xb1b0afba);
  assert.equal(checksum(result), 0xb1b0afba);
  assert.ok(tables(result).get('cmap').includes(Buffer.from([0x0e, 0x01, 0x00])));
  assert.equal(s.frees, 1);
  await Promise.all(s.pending);
  assert.equal((await s.serve(request(), s.context)).headers.get('x-cache'), 'HIT');
  assert.equal(s.reads, 1);
  s.revision('v2');
  assert.equal((await s.serve(request(), s.context)).headers.get('x-cache'), 'MISS');
  const other = createFontService(fonts, 'engine2', s.factory);
  assert.equal((await other(request(), s.context)).headers.get('x-cache'), 'MISS');
  const woff = await s.serve(request('Am', '&format=woff2'), s.context);
  assert.equal(woff.headers.get('content-type'), 'font/woff2');
  assert.equal(Buffer.from(await woff.arrayBuffer()).subarray(0, 4).toString(), 'wOF2');
  assert.equal(s.reads, 4);
});

test('invalid and rate-limited requests never open font sources; missing glyphs are explicit', async () => {
  const s = setup();
  for (const [req, status] of [
    [new Request(request(), { method: 'POST' }), 405],
    [new Request('https://ibis.test/api/font/subset?font=other&text=A'), 404],
    [request(''), 400], [request('A'.repeat(1025)), 400], [request('A', '&format=exe'), 400],
  ]) assert.equal((await s.serve(req, s.context)).status, status);
  s.rate(false);
  assert.equal((await s.serve(request(), s.context)).status, 429);
  assert.equal(s.reads, 0);
  s.rate(true);
  const missing = await s.serve(request('漢'), s.context);
  assert.equal(missing.status, 204);
  assert.equal(missing.headers.get('x-font-id'), 'test');
  assert.equal(s.frees, 1);
});

test('whole-file batches accept 1024 code points and reject overflow', async () => {
  const s = setup();
  assert.equal((await s.serve(request('A'.repeat(1024)), s.context)).status, 200);
  assert.equal((await s.serve(request('A'.repeat(1025)), s.context)).status, 400);
  // Even the worst-case percent-encoded supplementary characters fit the
  // service's 16 KiB URL budget at the permitted codepoint count.
  assert.ok(request('\u{100001}'.repeat(1024)).url.length < 16 * 1024);
});

test('truncated, oversized and failed streams release the engine and allow a later request', async () => {
  const s = setup();
  s.stream(bytes.slice(0, 100));
  assert.equal((await s.serve(request(), s.context)).status, 503);
  s.stream(new Uint8Array(bytes.length * 2));
  assert.equal((await s.serve(request(), s.context)).status, 503);
  assert.equal(s.canceled, true);
  s.stream(bytes);
  assert.equal((await s.serve(request(), s.context)).status, 200);
  assert.equal(s.frees, 3);
});

test('large font allocations are serialized without sharing request-owned I/O', async () => {
  const s = setup();
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  const original = s.context.store.read;
  let started;
  const ready = new Promise(resolve => { started = resolve; });
  s.context.store.read = async (...args) => { started(); await blocked; return original(...args); };
  const first = s.serve(request(), s.context);
  await ready;
  const busy = await s.serve(request('Bm'), s.context);
  assert.equal(busy.status, 429);
  assert.equal(busy.headers.get('retry-after'), '1');
  release();
  assert.equal((await first).status, 200);
  assert.equal(s.reads, 1);
});

test('an abandoned font lease is freed and a late stream cannot affect its replacement', async () => {
  const s = setup();
  let now = 0, controller, started;
  const ready = new Promise(resolve => { started = resolve; });
  const serve = createFontService(fonts, 'lease-engine', size => {
    const engine = s.factory(size), append = engine.append;
    return { ...engine, append(chunk) { append(chunk); started(); } };
  }, () => now);
  const original = s.context.store.read;
  s.context.store.read = async () => new ReadableStream({
    start(value) { controller = value; value.enqueue(bytes.slice(0, 100)); },
  });
  const abandoned = serve(request('Am'), s.context);
  await ready;
  now = 30_001;
  s.context.store.read = original;
  assert.equal((await serve(request('Bm'), s.context)).status, 200);
  assert.equal(s.frees, 2, 'expired and replacement engines each freed once');
  controller.enqueue(bytes.slice(100)); controller.close();
  assert.equal((await abandoned).status, 503);
  assert.equal(s.frees, 2, 'late completion does not double-free the old engine');
  assert.equal((await serve(request('Am'), s.context)).status, 200);
  assert.equal(s.frees, 3);
});
