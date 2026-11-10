// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { namespaceSubset, DynamicFonts } from '../../www/dynamic-fonts.js';
import { checksum, fontFamily } from '../../scripts/web-font-sfnt.mjs';

let cachedFixture;
async function fixture() {
  if (cachedFixture) return cachedFixture;
  const pkg = resolve(process.env.GJS_DIR, 'crates/font-subsetter/pkg');
  const bg = await import(pathToFileURL(join(pkg, 'font_subsetter_bg.js')).href);
  const { instance } = await WebAssembly.instantiate(await readFile(join(pkg, 'font_subsetter_bg.wasm')), { './font_subsetter_bg.js': bg });
  bg.__wbg_set_wasm(instance.exports);
  instance.exports.__wbindgen_start();
  const source = await readFile(new URL('../../crates/ibis-office-app/fonts/IBMPlexSans-Regular.ttf', import.meta.url));
  cachedFixture = bg.subset_font_raw(source, 'ABm');
  return cachedFixture;
}

test('dynamic gjs responses have independent families and valid checksums', { skip: !process.env.GJS_DIR }, async () => {
  const raw = await fixture();
  const first = namespaceSubset(raw, 1);
  const second = namespaceSubset(raw, 2);
  assert.equal(fontFamily(first), 'IBS 000001');
  assert.equal(fontFamily(second), 'IBS 000002');
  assert.equal(checksum(first), 0xb1b0afba);
  assert.equal(checksum(second), 0xb1b0afba);
  assert.throws(() => namespaceSubset(new Uint8Array([1, 2]), 1));
});

test('dynamic provider batches misses, splits unsupported batches and deduplicates', { skip: !process.env.GJS_DIR }, async () => {
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

test('a temporary network failure is retried', { skip: !process.env.GJS_DIR }, async () => {
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
