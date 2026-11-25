// SPDX-License-Identifier: AGPL-3.0-only
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { BRAND_NAME } from '../../www/brand.js';

const bootJs = readFileSync(new URL('../../www/boot.js', import.meta.url), 'utf8');
const bootTs = readFileSync(new URL('../../www/boot.ts', import.meta.url), 'utf8');
const mainTs = readFileSync(new URL('../../www/main.ts', import.meta.url), 'utf8');

test('www/brand.ts exports the canonical Web shell brand name', () => {
  assert.equal(BRAND_NAME, 'soraneDocs');
});

test('compiled www/boot.js runs as a classic script and formats the Japanese loading text with BRAND_NAME', () => {
  assert.doesNotMatch(bootJs, /^\s*(?:import|export)\b/m, 'boot.js must not emit ESM syntax');

  const statusText = { textContent: `Loading ${BRAND_NAME}…` };
  const doc = {
    documentElement: { lang: 'en' },
    getElementById: (id) => (id === 'status-text' ? statusText : null),
  };
  runInNewContext(bootJs, {
    location: { search: '' },
    URLSearchParams,
    localStorage: { getItem: () => null },
    navigator: { language: 'ja-JP' },
    document: doc,
  });
  assert.equal(doc.documentElement.lang, 'ja');
  assert.equal(statusText.textContent, `${BRAND_NAME} を読み込んでいます…`);
});

test('compiled www/boot.js leaves non-Japanese loading text untouched', () => {
  const initial = `Loading ${BRAND_NAME}…`;
  const statusText = { textContent: initial };
  const doc = {
    documentElement: { lang: 'en' },
    getElementById: (id) => (id === 'status-text' ? statusText : null),
  };
  runInNewContext(bootJs, {
    location: { search: '' },
    URLSearchParams,
    localStorage: { getItem: () => null },
    navigator: { language: 'en-US' },
    document: doc,
  });
  assert.equal(doc.documentElement.lang, 'en');
  assert.equal(statusText.textContent, initial);
});

test('startup status strings in www/main.ts and www/boot.ts use the shared brand constant', () => {
  assert.match(mainTs, /import \{ BRAND_NAME \} from '\.\/brand\.js';/);
  const startupMatches = mainTs.match(/`\$\{BRAND_NAME\} を起動しています…` : `Starting \$\{BRAND_NAME\}…`/g) ?? [];
  assert.equal(startupMatches.length, 2);
  assert.doesNotMatch(mainTs, /(?:ibisOffice|Docs)\s*を起動しています|Starting\s+(?:ibisOffice|Docs)/);
  assert.doesNotMatch(bootTs, /(?:ibisOffice|Docs)\s*を読み込んでいます|Loading\s+(?:ibisOffice|Docs)/);
});
