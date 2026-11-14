// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { serveBrotliWasm } from '../../worker/wasm-delivery.js';

const hash = 'a'.repeat(64);
const url = `https://example.test/pkg/br/${hash}.wasm`;

test('precompressed Wasm is streamed only when Brotli is accepted', async () => {
  const calls = [];
  const assets = { async fetch(request) {
    calls.push({ url: request.url, accept: request.headers.get('accept-encoding') });
    return new Response(new Uint8Array([1, 2, 3]), { headers: { 'Content-Type': 'application/octet-stream' } });
  } };
  for (const accept of ['gzip, br', 'br;q=0.5', 'gzip, *;q=0.1']) {
    const result = await serveBrotliWasm(new Request(url, { headers: { 'Accept-Encoding': accept } }), assets);
    assert.equal(result.status, 200);
    assert.equal(result.headers.get('Content-Encoding'), 'br');
    assert.equal(result.headers.get('Content-Type'), 'application/wasm');
    assert.equal(result.headers.get('Vary'), 'Accept-Encoding');
    assert.deepEqual(new Uint8Array(await result.arrayBuffer()), new Uint8Array([1, 2, 3]));
  }
  assert.ok(calls.every(call => call.url.endsWith(`/pkg/wasm-br.${hash}.bin`) && call.accept === 'identity'));
  const count = calls.length;
  for (const accept of ['gzip', 'br;q=0', 'br;q=0,*;q=1', 'br;q=wat']) {
    const result = await serveBrotliWasm(new Request(url, { headers: { 'Accept-Encoding': accept } }), assets);
    assert.equal(result.status, 406);
    assert.equal(result.headers.get('Cache-Control'), 'no-store');
  }
  const rewritten = new Request(url, { headers: { 'Accept-Encoding': 'br, gzip' } });
  Object.defineProperty(rewritten, 'cf', { value: { clientAcceptEncoding: 'gzip' } });
  assert.equal((await serveBrotliWasm(rewritten, assets)).status, 406);
  assert.equal(calls.length, count);
  const head = await serveBrotliWasm(new Request(url, { method: 'HEAD', headers: { 'Accept-Encoding': 'br' } }), assets);
  assert.equal(head.status, 200);
  assert.equal(head.body, null);
  assert.equal(await serveBrotliWasm(new Request('https://example.test/other'), assets), null);
  assert.equal((await serveBrotliWasm(new Request(url, { method: 'POST' }), assets)).status, 405);
});

test('missing and unexpectedly encoded assets are not served as Brotli', async () => {
  const request = new Request(url, { headers: { 'Accept-Encoding': 'br' } });
  assert.equal((await serveBrotliWasm(request, { fetch: async () => new Response('', { status: 404 }) })).status, 404);
  assert.equal((await serveBrotliWasm(request, { fetch: async () => new Response('', { headers: { 'Content-Encoding': 'gzip' } }) })).status, 502);
});
