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
