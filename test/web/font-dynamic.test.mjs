// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { namespaceSubset, DynamicFonts } from '../../www/dynamic-fonts.js';
import { checksum, fontFamily } from '../../scripts/web-font-sfnt.mjs';

// Produced by the gjs subsetter from the bundled IBM Plex Sans test font.
const fixture = () => readFile(new URL('./fixtures/gjs-plex-abm.otf', import.meta.url));

test('dynamic gjs responses have independent families and valid checksums', async () => {
  const raw = await fixture();
  const first = namespaceSubset(raw, 1);
  const second = namespaceSubset(raw, 2);
  assert.equal(fontFamily(first), 'IBS 000001');
  assert.equal(fontFamily(second), 'IBS 000002');
  assert.equal(checksum(first), 0xb1b0afba);
  assert.equal(checksum(second), 0xb1b0afba);
  assert.throws(() => namespaceSubset(new Uint8Array([1, 2]), 1));
});

test('dynamic provider batches misses, splits unsupported batches and deduplicates', async () => {
  const raw = await fixture();
  const calls = [], installed = [];
  let complete;
  const done = new Promise(resolve => { complete = resolve; });
  const provider = new DynamicFonts({ endpoint: 'https://example.test/api/font/subset', font: 'TestFont' }, data => {
    installed.push(fontFamily(data));
    if (installed.length === 2) complete();
  }, async url => {
    const text = url.searchParams.get('text');
    calls.push(text);
    assert.equal(url.searchParams.get('format'), 'otf');
    return text.length > 2 ? new Response(null, { status: 204 }) : new Response(raw);
  });
  provider.request(['A', 'B', 'A']);
  await Promise.race([done, new Promise((_, reject) => setTimeout(() => reject(new Error('Timed out')), 3000))]);
  assert.deepEqual(calls, ['ABm', 'Am', 'Bm']);
  assert.deepEqual(installed, ['IBS 000001', 'IBS 000002']);
  provider.request(['A', 'B']);
  assert.equal(calls.length, 3);
});

test('a temporary network failure is retried', async () => {
  const raw = await fixture();
  let calls = 0;
  let complete;
  const done = new Promise(resolve => { complete = resolve; });
  const provider = new DynamicFonts({ endpoint: 'https://example.test/api/font/subset', font: 'TestFont' }, () => complete(), async () => {
    calls++;
    if (calls === 1) throw new TypeError('temporary failure');
    return new Response(raw);
  });
  provider.request(['A']);
  await Promise.race([done, new Promise((_, reject) => setTimeout(() => reject(new Error('Timed out')), 3000))]);
  assert.equal(calls, 2);
});

test('face subsets keep requested fonts separate and omit Latin from administrative requests', async () => {
  const { FaceSubsetProvider } = await import('../../www/dynamic-fonts.js');
  const raw = await fixture(), calls = [], installed = [];
  let complete;
  const done = new Promise(resolve => { complete = resolve; });
  const pua = String.fromCodePoint(0x100000);
  const provider = new FaceSubsetProvider('https://example.test/subset', (data, face) => {
    installed.push([face, fontFamily(data)]);
    if (['acgjm', 'ipamjm', 'NotoSansCJKJP'].every(face => installed.some(([name]) => name === face))) complete();
  }, async url => {
    const face = url.searchParams.get('font'), text = url.searchParams.get('text');
    calls.push([face, text]);
    if (face === 'acgjm') assert.equal(text, pua);
    // A mixed batch fallback must be split so the ordinary character retains its requested face.
    const actual = text.includes('A') && text.includes(pua) ? 'Jigmo' : face;
    return new Response(raw, { headers: { 'X-Font-Id': actual } });
  });
  provider.request(['A', pua, 'A']);
  await Promise.race([done, new Promise((_, reject) => setTimeout(() => reject(new Error('Timed out')), 3000))]);
  assert.ok(calls.some(([face, text]) => face === 'ipamjm' && text === 'Am'));
  assert.ok(calls.some(([face, text]) => face === 'NotoSansCJKJP' && text === 'Am'));
  assert.ok(installed.some(([face]) => face === 'acgjm'));
  assert.ok(installed.findIndex(([face]) => face === 'NotoSansCJKJP') < installed.findIndex(([face]) => face === 'ipamjm'));
  assert.ok(!calls.some(([face, text]) => face !== 'acgjm' && text.includes(pua)));
  assert.equal(new Set(installed.map(([, family]) => family)).size, installed.length);
});
