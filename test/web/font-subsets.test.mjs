// SPDX-License-Identifier: AGPL-3.0-only
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULT_FONT_IDS,
  TYPEFACE_ALIASES,
  buildLoaderManifest,
  parseFontList,
  readWasmVersion,
} from '../../scripts/build-web-font-subsets.mjs';

test('docs/font-list.txt contains all default SIL OFL 1.1 subset fonts', () => {
  const text = readFileSync(new URL('../../docs/font-list.txt', import.meta.url), 'utf8');
  const entries = parseFontList(text);
  const byId = new Map(entries.map((e) => [e.id, e]));
  for (const id of DEFAULT_FONT_IDS) {
    const entry = byId.get(id);
    assert.ok(entry, `missing ${id} in docs/font-list.txt`);
    assert.equal(entry.license, 'SIL OFL 1.1', `${id} must be SIL OFL 1.1`);
    assert.ok(TYPEFACE_ALIASES[id]?.length > 0, `missing typeface aliases for ${id}`);
  }
});

test('buildLoaderManifest emits a schema-1 subsets-only manifest for subset-fetch.ts', () => {
  const versionText = readFileSync(new URL('../../version.txt', import.meta.url), 'utf8');
  const appVersion = readWasmVersion(versionText);
  assert.match(appVersion, /^\d+\.\d+\.\d+/);

  const manifest = buildLoaderManifest(appVersion, [
    {
      id: 'ACGJMincho',
      typeface: TYPEFACE_ALIASES.ACGJMincho,
      license: 'SIL OFL 1.1',
      licenseFile: 'ACGJMincho/OFL.txt',
      blocks: [
        {
          key: 'base',
          family: 'ibis ACGJMincho base',
          file: 'ACGJMincho/base.abc.ttf',
          size: 1234,
          deflatedSize: 567,
          sha256: 'a'.repeat(64),
          ranges: [[0x20, 0x7e]],
          boot: true,
        },
        {
          key: '0001',
          family: 'ibis ACGJMincho 0001',
          file: 'ACGJMincho/0001.def.ttf',
          size: 890,
          deflatedSize: 400,
          sha256: 'b'.repeat(64),
          ranges: [[0xf0000, 0xf0fff]],
          boot: false,
        },
      ],
    },
  ]);

  assert.equal(manifest.schema, 1);
  assert.equal(manifest.app, appVersion);
  assert.deepEqual(manifest.files, []);
  assert.equal(manifest.subsets.schema, 1);
  assert.equal(manifest.subsets.fonts.length, 1);
  const font = manifest.subsets.fonts[0];
  assert.equal(font.licenseFile, 'fonts/ACGJMincho/OFL.txt');
  assert.equal(font.blocks[0].file, 'fonts/ACGJMincho/base.abc.ttf');
  assert.equal(font.blocks[0].boot, true);
  assert.equal(font.blocks[1].file, 'fonts/ACGJMincho/0001.def.ttf');
  assert.equal('boot' in font.blocks[1], false);
});

test('TYPEFACE_ALIASES covers common Japanese mincho and gothic names without cross-class mixing', () => {
  for (const name of ['BIZ UD明朝', 'BIZ UDP明朝', '游明朝', 'ＭＳ 明朝', 'ＭＳ明朝', 'ヒラギノ明朝 ProN', 'Noto Serif JP', 'Noto Serif CJK JP']) {
    assert.ok(TYPEFACE_ALIASES.BIZUDMincho.includes(name), `BIZUDMincho missing ${name}`);
  }
  for (const name of ['BIZ UDゴシック', 'BIZ UDPゴシック', '游ゴシック', 'ＭＳ ゴシック', 'ＭＳゴシック', 'Noto Sans JP', 'Noto Sans CJK JP']) {
    assert.ok(TYPEFACE_ALIASES.BIZUDGothic.includes(name), `BIZUDGothic missing ${name}`);
  }
});

test('Web CI release build enables IBIS_WEB_FONT_SUBSETS=1', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/web-fonts.yml', import.meta.url), 'utf8');
  assert.match(workflow, /IBIS_WEB_FONT_SUBSETS:\s*['"]?1['"]?/);
});
