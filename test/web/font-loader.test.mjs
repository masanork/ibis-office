// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { loadWebFonts } from '../../www/font-loader.js';

const bytes = await readFile(new URL('../../crates/ibis-office-app/fonts/IBMPlexSans-Regular.ttf', import.meta.url));
const manifestURL = new URL('https://example.test/editor/fonts.json');
const manifest = () => ({
  version: 1, defaultFamily: 'IBM Plex Sans', browserFallback: false,
  fonts: [{ url: './assets/fonts/plex.ttf', family: 'IBM Plex Sans', aliases: ['Office Font'], sha256: createHash('sha256').update(bytes).digest('hex') }],
});

test('only an absent manifest enables legacy browser fonts', async () => {
  assert.equal(await loadWebFonts(manifestURL, async () => new Response('', { status: 404 })), null);
  await assert.rejects(loadWebFonts(manifestURL, async () => new Response('', { status: 503 })), /HTTP 503/);
});

test('verifies binary fonts under a deployment subpath before supplying aliases', async () => {
  const requests = [];
  const fonts = await loadWebFonts(manifestURL, async (url, options) => {
    requests.push([url.href, options]);
    return url.href === manifestURL.href ? Response.json(manifest()) : new Response(bytes);
  });
  assert.equal(requests[0][1].cache, 'no-cache');
  assert.equal(requests[1][0], 'https://example.test/editor/assets/fonts/plex.ttf');
  assert.deepEqual(fonts.data[0], new Uint8Array(bytes));
  assert.equal(fonts.aliases['Office Font'], 'IBM Plex Sans');
  assert.equal(fonts.browserFallback, false);
  assert.equal(fonts.uiFamily, fonts.defaultFamily);
});

test('loads an independent UI face without changing document aliases', async () => {
  const carlito = await readFile(new URL('../../crates/ibis-office-render/fonts/carlito/Carlito-Regular.ttf', import.meta.url));
  const value = manifest();
  value.defaultFamily = 'Carlito';
  value.uiFamily = 'IBM Plex Sans';
  value.fonts.push({ url: './assets/fonts/carlito.ttf', family: 'Carlito', aliases: ['MS Mincho'], sha256: createHash('sha256').update(carlito).digest('hex') });
  const loaded = await loadWebFonts(manifestURL, async url => url.href === manifestURL.href
    ? Response.json(value) : new Response(url.pathname.endsWith('carlito.ttf') ? carlito : bytes));
  assert.equal(loaded.uiFamily, 'IBM Plex Sans');
  assert.equal(loaded.defaultFamily, 'Carlito');
  assert.equal(loaded.aliases['MS Mincho'], 'Carlito');
  assert.deepEqual(loaded.data[1], new Uint8Array(carlito));
});

test('corrupt data and HTML/WOFF responses cannot silently use browser fonts', async () => {
  const corrupt = Buffer.from(bytes);
  corrupt[corrupt.length - 1] ^= 1;
  for (const [data, message] of [[corrupt, /SHA-256/], [Buffer.from('<html>missing font</html>'), /TTF\/OTF/]]) {
    await assert.rejects(loadWebFonts(manifestURL, async url => url.href === manifestURL.href ? Response.json(manifest()) : new Response(data)), message);
  }
  await assert.rejects(loadWebFonts(manifestURL, async url => url.href === manifestURL.href ? Response.json(manifest()) : new Response('', { status: 404 })), /HTTP 404/);
});

test('rejects ambiguous aliases and unknown defaults before fetching fonts', async () => {
  for (const change of [
    m => { m.fonts.push({ ...m.fonts[0], family: 'Other Family' }); },
    m => { m.defaultFamily = 'Missing'; },
    m => { m.uiFamily = 'Missing'; },
    m => { m.fonts[0].sha256 = 'bad'; },
    m => { m.browserFallback = 'false'; },
    m => { m.version = 2; },
  ]) {
    const value = manifest();
    change(value);
    let calls = 0;
    await assert.rejects(loadWebFonts(manifestURL, async () => { calls++; return Response.json(value); }));
    assert.equal(calls, 1);
  }
});

test('validates optional dynamic endpoint and requires strict font fallback', async () => {
  const value = manifest();
  value.dynamic = { endpoint: '/api/font/subset', font: 'ipamjm' };
  const fetcher = async url => url.href === manifestURL.href ? Response.json(value) : new Response(bytes);
  const loaded = await loadWebFonts(manifestURL, fetcher);
  assert.equal(loaded.dynamic.font, 'ipamjm');
  assert.equal(loaded.dynamic.endpoint, 'https://example.test/api/font/subset');
  value.browserFallback = true;
  await assert.rejects(loadWebFonts(manifestURL, fetcher), /dynamic font/);
  value.browserFallback = false;
  value.dynamic.endpoint = 'javascript:alert(1)';
  await assert.rejects(loadWebFonts(manifestURL, fetcher), /dynamic font/);
});

test('loads and verifies ordered font chunks and rejects corrupt data', async () => {
  const parts = [bytes.subarray(0, 4000), bytes.subarray(4000)];
  const hash = data => createHash('sha256').update(data).digest('hex');
  const value = manifest();
  delete value.fonts[0].url;
  value.fonts[0].size = bytes.length;
  value.fonts[0].parts = parts.map((data, i) => ({ url: `./assets/fonts/${i}.fontchunk`, size: data.length, sha256: hash(data) }));
  const fetcher = async url => url.href === manifestURL.href ? Response.json(value) : new Response(parts[Number(url.pathname.match(/(\d)\.fontchunk$/)[1])]);
  const loaded = await loadWebFonts(manifestURL, fetcher);
  assert.deepEqual(loaded.data[0], new Uint8Array(bytes));
  value.fonts[0].parts.reverse();
  await assert.rejects(loadWebFonts(manifestURL, fetcher), /TTF\/OTF|SHA-256/);
  value.fonts[0].parts.reverse();
  value.fonts[0].parts[0].sha256 = '0'.repeat(64);
  await assert.rejects(loadWebFonts(manifestURL, fetcher), /invalid part/);
  value.fonts[0].parts[0].sha256 = hash(parts[0]);
  value.fonts[0].size++;
  await assert.rejects(loadWebFonts(manifestURL, fetcher), /font size/);
});
