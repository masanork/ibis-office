// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { uiRepertoire } from '../../scripts/ui-font-repertoire.mjs';
import { buildWebFonts } from '../../scripts/build-web-fonts.mjs';
import { loadSubsetter } from '../../scripts/font-subsetter.mjs';

async function fixture(t) {
  const dir = await mkdtemp(join(tmpdir(), 'ibis-ui-font-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await mkdir(join(dir, 'ja'));
  return dir;
}

test('UI repertoire reads display values, keeps IVS/emoji clusters and follows resource additions', async t => {
  const dir = await fixture(t);
  const tsv = join(dir, 'ja', 'ui.tsv'), yml = join(dir, 'ui.yml');
  await writeFile(tsv, '# COMMENT漢\nignored鍵\t編集A\u{E0100}🔁️\nagain\t編集\n');
  await writeFile(yml, '_version: 2\nMenu:\n  open:\n    en: Open\n    ja: "開く"\n    zh-CN: 忽略\n');
  const required = await uiRepertoire([join(dir, 'ja'), yml]);
  assert.deepEqual(required.slice(0, 4), ['編', '集', 'A\u{E0100}', '🔁️']);
  assert.ok(required.includes('開'));
  assert.ok(!required.some(s => /[鍵漢忽略]/u.test(s)));
  await writeFile(tsv, 'new\t追記\n');
  assert.deepEqual(await uiRepertoire([join(dir, 'ja')]), ['追', '記']);
});

test('real UI resource inventory includes Japanese menus, status text and shortcut symbols', async () => {
  const required = await uiRepertoire();
  for (const glyph of '編集表示文字数⌃🔁') assert.ok(required.some(s => s.includes(glyph)), glyph);
  assert.ok(required.length > 1000);
});

test('bootstrap subsets cover the resource union and reject unsupported new UI text before publishing', async t => {
  const dir = await fixture(t), tsv = join(dir, 'ja', 'ui.tsv');
  const plex = fileURLToPath(new URL('../../crates/ibis-office-app/fonts/IBMPlexSans-Regular.ttf', import.meta.url));
  const synthetic = fileURLToPath(new URL('../../crates/ibis-font-subsetter/tests/fonts/ivs.ttf', import.meta.url));
  await writeFile(tsv, 'label\tA\u{100001}\n');
  const config = join(dir, 'config.json');
  await writeFile(config, JSON.stringify({ uiFont: 'ui', fonts: [
    { id: 'ui', path: plex, text: ' Am', uiResources: true },
    { id: 'symbols', path: synthetic, uiResources: true },
  ] }));
  const output = join(dir, 'www');
  const manifest = await buildWebFonts(config, { outDir: output, uiSources: [join(dir, 'ja')] });
  assert.equal(manifest.uiFamily, 'Ibis Subset ui');
  const bg = await loadSubsetter();
  const fonts = await Promise.all(manifest.fonts.map(f => readFile(new URL(f.url, `file://${output}/fonts.json`))));
  assert.ok(bg.font_has_glyphs(fonts[0], 'A'));
  assert.ok(!bg.font_has_glyphs(fonts[0], '\u{100001}'));
  assert.ok(bg.font_has_glyphs(fonts[1], '\u{100001}'));
  const before = await readFile(join(output, 'fonts.json'));
  await writeFile(tsv, 'new\t漢\n');
  await assert.rejects(buildWebFonts(config, { outDir: output, uiSources: [join(dir, 'ja')] }), /UI resources lack bootstrap glyphs: U\+6F22/);
  assert.deepEqual(await readFile(join(output, 'fonts.json')), before);
});
