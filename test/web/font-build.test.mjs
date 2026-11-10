// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildWebFonts } from '../../scripts/build-web-fonts.mjs';
import { checksum, fontFamily, renameSubset, tables } from '../../scripts/web-font-sfnt.mjs';

const source = fileURLToPath(new URL('../../crates/ibis-office-app/fonts/IBMPlexSans-Regular.ttf', import.meta.url));
const bytes = await readFile(source);

test('renaming keeps shaping, glyph, and license data, with valid SFNT checksums', () => {
  const renamed = renameSubset(bytes, 'Ibis Subset Test');
  assert.equal(fontFamily(renamed), 'Ibis Subset Test');
  const before = tables(bytes);
  const after = tables(renamed);
  for (const [tag, data] of before) {
    if (!['name', 'head', 'DSIG'].includes(tag)) assert.deepEqual(after.get(tag), data, tag);
  }
  assert.equal(checksum(renamed), 0xb1b0afba);
  // Name ID 13 (license description) and copyright remain in the name table.
  for (const text of ['Copyright', 'SIL Open Font License']) {
    assert.equal(after.get('name').includes(Buffer.from(text, 'utf16le').swap16()), before.get('name').includes(Buffer.from(text, 'utf16le').swap16()));
  }
});

test('full font mode needs no gjs checkout and preserves the original bytes', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-font-full-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const config = join(dir, 'fonts.json');
  await writeFile(config, JSON.stringify({ fonts: [{ id: 'plex', path: source, aliases: ['Office Font'] }] }));
  const manifest = await buildWebFonts(config, { gjsDir: '/no/gjs', outDir: join(dir, 'www') });
  assert.equal(manifest.defaultFamily, 'IBM Plex Sans');
  assert.equal(manifest.uiFamily, 'IBM Plex Sans');
  assert.equal(manifest.browserFallback, false);
  assert.deepEqual(await readFile(new URL(manifest.fonts[0].url, `file://${dir}/www/fonts.json`)), bytes);
});

test('invalid default and duplicate aliases leave the previous manifest intact', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-font-invalid-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const config = join(dir, 'config.json');
  const output = join(dir, 'fonts.json');
  await writeFile(output, 'previous');
  await writeFile(config, JSON.stringify({ defaultFont: 'missing', fonts: [{ id: 'plex', path: source }] }));
  await assert.rejects(buildWebFonts(config, { outDir: dir }), /defaultFont/);
  assert.equal(await readFile(output, 'utf8'), 'previous');
  await writeFile(config, JSON.stringify({ uiFont: 'missing', fonts: [{ id: 'plex', path: source }] }));
  await assert.rejects(buildWebFonts(config, { outDir: dir }), /uiFont/);
  assert.equal(await readFile(output, 'utf8'), 'previous');
  const carlito = fileURLToPath(new URL('../../crates/ibis-office-app/fonts/carlito/Carlito-Regular.ttf', import.meta.url));
  await writeFile(config, JSON.stringify({ fonts: [
    { id: 'plex', path: source, aliases: ['Shared Alias'] },
    { id: 'carlito', path: carlito, aliases: ['Shared Alias'] },
  ] }));
  await assert.rejects(buildWebFonts(config, { outDir: dir }), /Conflicting font alias/);
  assert.equal(await readFile(output, 'utf8'), 'previous');
});

test('UI selection preserves a different document default and named aliases', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-font-ui-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const config = join(dir, 'fonts.json');
  const carlito = fileURLToPath(new URL('../../crates/ibis-office-app/fonts/carlito/Carlito-Regular.ttf', import.meta.url));
  await writeFile(config, JSON.stringify({ defaultFont: 'document', uiFont: 'ui', fonts: [
    { id: 'document', path: carlito, aliases: ['Document Face'] },
    { id: 'ui', path: source },
  ] }));
  const manifest = await buildWebFonts(config, { outDir: join(dir, 'www') });
  assert.equal(manifest.defaultFamily, 'Carlito');
  assert.equal(manifest.uiFamily, 'IBM Plex Sans');
  assert.deepEqual(manifest.fonts[0].aliases, ['Carlito', 'Document Face']);
});

// Opt in to the sibling gjs checkout; normal CI has no external font dependency.
test('gjs subsets two distinct fonts without a family collision', { skip: !process.env.GJS_DIR }, async t => {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-font-gjs-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const config = join(dir, 'config.json');
  const carlito = fileURLToPath(new URL('../../crates/ibis-office-app/fonts/carlito/Carlito-Regular.ttf', import.meta.url));
  await writeFile(join(dir, 'text.txt'), 'Hello office 123\n');
  await writeFile(config, JSON.stringify({ fonts: [
    { id: 'plex', path: source, textFile: 'text.txt' },
    { id: 'carlito', path: carlito, textFile: 'text.txt', aliases: ['Calibri'] },
  ] }));
  const manifest = await buildWebFonts(config, { gjsDir: process.env.GJS_DIR, outDir: dir });
  assert.notEqual(manifest.fonts[0].family, manifest.fonts[1].family);
  for (const font of manifest.fonts) {
    const data = await readFile(new URL(font.url, `file://${dir}/fonts.json`));
    assert.equal(fontFamily(data), font.family);
    assert.equal(checksum(data), 0xb1b0afba);
    assert.ok(data.length < bytes.length / 2);
  }
});

test('gjs retains both BMP and supplementary variation selectors', { skip: !process.env.GJS_DIR }, async t => {
  const sourceFont = join(process.env.GJS_DIR, 'data/sources/fonts/ipamjm.ttf');
  try { await readFile(sourceFont); } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    t.skip('Optional IPAmj font is not downloaded');
    return;
  }
  const dir = await mkdtemp(join(tmpdir(), 'ibis-font-ivs-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const config = join(dir, 'config.json');
  await writeFile(config, JSON.stringify({ fonts: [
    { id: 'ivs', path: resolve(sourceFont), text: '侮\uFE00侮\u{E0103}侮' },
  ] }));
  const manifest = await buildWebFonts(config, { gjsDir: process.env.GJS_DIR, outDir: dir });
  const data = await readFile(new URL(manifest.fonts[0].url, `file://${dir}/fonts.json`));
  const cmap = tables(data).get('cmap');
  const variants = new Map();
  for (let i = 0; i < cmap.readUInt16BE(2); i++) {
    const offset = cmap.readUInt32BE(4 + i * 8 + 4);
    if (cmap.readUInt16BE(offset) !== 14) continue;
    for (let j = 0; j < cmap.readUInt32BE(offset + 6); j++) {
      const record = offset + 10 + j * 11;
      const selector = cmap.readUIntBE(record, 3);
      const nondefault = cmap.readUInt32BE(record + 7);
      if (!nondefault) continue;
      const start = offset + nondefault;
      for (let k = 0; k < cmap.readUInt32BE(start); k++) {
        const at = start + 4 + k * 5;
        if (cmap.readUIntBE(at, 3) === 0x4fae) variants.set(selector, cmap.readUInt16BE(at + 3));
      }
    }
  }
  assert.ok(variants.get(0xfe00) > 0);
  assert.ok(variants.get(0xe0103) > 0);
  assert.equal(checksum(data), 0xb1b0afba);
});

test('large full fonts are divided into verified assets without altering the source', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-font-large-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const large = Buffer.alloc(20 * 1024 * 1024 + 127);
  bytes.copy(large);
  const input = join(dir, 'large.ttf'), config = join(dir, 'config.json');
  await writeFile(input, large);
  await writeFile(config, JSON.stringify({ fonts: [{ id: 'large', path: input }] }));
  const result = await buildWebFonts(config, { outDir: dir });
  const font = result.fonts[0];
  assert.equal(font.parts.length, 2);
  assert.equal(font.size, large.length);
  const output = Buffer.concat(await Promise.all(font.parts.map(part => readFile(new URL(part.url, `file://${dir}/fonts.json`)))));
  assert.deepEqual(output, large);
});
