// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fontCoverage } from '../../www/font-coverage.js';
import { documentGlyphs, DynamicFonts, FaceSubsetProvider } from '../../www/dynamic-fonts.js';
import { loadSubsetter } from '../../scripts/font-subsetter.mjs';

const bg = await loadSubsetter();
const source = await readFile(new URL('../../crates/ibis-font-subsetter/tests/fonts/ivs.ttf', import.meta.url));
const raw = bg.subset_font_raw(source, 'ABmA\u{FE00}A\u{E0100}\u{100001}');

test('bootstrap coverage reads BMP, supplementary and actual variation mappings', async () => {
  for (const path of ['ivs.ttf', 'ivs-cff.otf']) {
    const bytes = await readFile(new URL('../../crates/ibis-font-subsetter/tests/fonts/' + path, import.meta.url));
    const covers = fontCoverage(bytes);
    for (const text of ['A', 'A\uFE00', 'A\u{E0100}', '\u{100001}', '\u{20000}']) assert.ok(covers(text), text);
    for (const text of ['Z', 'A\u{E0101}', '\u{20001}']) assert.ok(!covers(text), text);
  }
  const plex = fontCoverage(await readFile(new URL('../../crates/ibis-office-app/fonts/IBMPlexSans-Regular.ttf', import.meta.url)));
  assert.ok(plex('Azm'));
  assert.ok(!plex('A\u{E0100}'));
  assert.ok(!plex('漢'));
});

test('whole-file prefetch retains distinct graphemes and excludes controls', () => {
  assert.deepEqual(documentGlyphs('葛\u{E0100}\n葛\u{E0101}\r\n葛\u{E0100}e\u0301\t\u200e'), ['葛\u{E0100}', '葛\u{E0101}', 'e\u0301']);
});

test('prefetch skips bootstrap per face, fetches unseen tail characters and waits for installation', async () => {
  const calls = [], installed = [];
  const pua = '\u{100001}';
  const provider = new FaceSubsetProvider('https://ibis.test/api/font/subset', (_data, face) => installed.push(face), async url => {
    const font = url.searchParams.get('font'), text = url.searchParams.get('text'); calls.push([font, text]);
    await new Promise(resolve => setTimeout(resolve, 5));
    return new Response(raw, { headers: { 'X-Font-Id': font } });
  });
  provider.seed('NotoSansCJKJP', glyph => glyph === 'A');
  provider.seed('ipamjm', glyph => glyph === 'B');
  await provider.preload('A\nB\n' + pua + 'A');
  assert.deepEqual(calls, [['acgjm', pua], ['NotoSansCJKJP', 'Bm'], ['ipamjm', 'Am']]);
  assert.equal(installed.length, 3);
  provider.request(['A', 'B', pua]);
  await provider.preload('A\nB\n' + pua);
  assert.equal(calls.length, 3, 'scrolling/reopening does not request preloaded text again');
});

test('file repertoire batches beyond the viewport while respecting the API and keeping clusters intact', async () => {
  const calls = [];
  const glyphs = Array.from({ length: 1200 }, (_, i) => String.fromCodePoint(0x4e00 + i));
  glyphs.push('A\u{E0100}');
  const provider = new FaceSubsetProvider('https://ibis.test/api/font/subset', () => {}, async url => {
    const font = url.searchParams.get('font'), text = url.searchParams.get('text');
    assert.ok([...text].length <= 1024);
    calls.push([font, text]);
    return new Response(raw, { headers: { 'X-Font-Id': font } });
  });
  await provider.preload(glyphs.join('\n'));
  for (const face of ['NotoSansCJKJP', 'ipamjm']) {
    const all = calls.filter(([font]) => font === face).map(([,text]) => text.slice(0, -1)).join('');
    for (const glyph of glyphs) assert.ok(all.includes(glyph), glyph);
  }
  assert.equal(calls.length, 4);
});

test('prefetch failures are explicit and can be retried rather than marked as loaded', async () => {
  let fail = true, count = 0;
  const provider = new FaceSubsetProvider('https://ibis.test/api/font/subset', () => {}, async url => {
    count++;
    if (fail) return new Response(null, { status: 503 });
    return new Response(raw, { headers: { 'X-Font-Id': url.searchParams.get('font') } });
  });
  await assert.rejects(provider.preload('A'), /Document fonts could not be loaded/);
  assert.equal(count, 3);
  fail = false;
  await provider.preload('A');
  assert.equal(count, 5);
});

test('large prefetch batches adapt to older endpoints with smaller text limits', async () => {
  const supplied = new Map();
  const glyphs = Array.from({ length: 180 }, (_, i) => String.fromCodePoint(0x4e00 + i));
  const provider = new FaceSubsetProvider('https://legacy.test/api/font/subset', () => {}, async url => {
    const face = url.searchParams.get('font'), text = url.searchParams.get('text');
    if ([...text].length > 128) return new Response(null, { status: 400 });
    supplied.set(face, (supplied.get(face) ?? '') + text);
    return new Response(raw, { headers: { 'X-Font-Id': face } });
  });
  await provider.preload(glyphs.join(''));
  for (const face of ['NotoSansCJKJP', 'ipamjm']) for (const glyph of glyphs) assert.ok(supplied.get(face).includes(glyph));
});

test('legacy fallback provider also awaits whole-file loading and skips supplied glyphs', async () => {
  const calls = [];
  const provider = new DynamicFonts({ endpoint: 'https://ibis.test/api/font/subset', font: 'test' }, () => {}, async url => {
    calls.push(url.searchParams.get('text')); return new Response(raw);
  });
  provider.seed(glyph => glyph === 'A');
  await provider.preload('ABBA');
  assert.deepEqual(calls, ['Bm']);
});
