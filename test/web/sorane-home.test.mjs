// SPDX-License-Identifier: AGPL-3.0-only
import { readFileSync, existsSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';

const home = readFileSync(new URL('../../www/index.html', import.meta.url), 'utf8');
const editor = readFileSync(new URL('../../www/editor.html', import.meta.url), 'utf8');
const startScript = readFileSync(new URL('../../www/sorane-start.js', import.meta.url), 'utf8');

test('index.html boots the WASM app directly with start-view and GitHub repo integration', () => {
  assert.match(home, /<script src="\.\/boot\.js"><\/script>/);
  assert.match(home, /<script src="\.\/sorane-start\.js"><\/script>/);
  assert.match(home, /<script type="module" src="\.\/main\.js"><\/script>/);
  assert.match(home, /<script type="module" src="\.\/sorane-github-vault\.js"><\/script>/);
  assert.match(editor, /<script src="\.\/sorane-start\.js"><\/script>/);
});

test('index.html and editor.html disable pointer events on the hidden IME textarea', () => {
  assert.match(home, /body\s*>\s*textarea\s*\{[^}]*pointer-events:\s*none/);
  assert.match(editor, /body\s*>\s*textarea\s*\{[^}]*pointer-events:\s*none/);
});

test('sorane-start.js configures the featured samples and all bundled files exist', () => {
  const win = {};
  runInNewContext(startScript, {
    window: win,
    location: { search: '' },
    URLSearchParams,
  });
  const startView = win.ibisOfficeOptions?.startView;
  assert.ok(startView, 'startView config should be installed');
  assert.equal(startView.items.length, 10);
  assert.equal(startView.selectAction?.action, 'github-vault');
  for (const name of [
    'marp-deck-sample.md',
    'workbook-sheet-sample.md',
    'sample-ordinance.lawtext',
    'roster-matching-sample.xlsx',
    'mermaid-showcase.md',
    'mynumber-law-2023-comparison.jtd',
    'urban-comfort-analysis.xlsx',
    'standard-linkage.csv',
    'visio-multi-page-handbook.vsdx',
    'ibis-office-wasm.sbom.cdx.json',
  ]) {
    assert.ok(startView.items.some((item) => item.url === `/samples/${name}`), `missing sample ${name}`);
    assert.ok(existsSync(new URL(`../../www/samples/${name}`, import.meta.url)), `file missing: ${name}`);
  }
});

test('sorane-start.js stays out of the way in host mode', () => {
  const hostWin = {};
  runInNewContext(startScript, {
    window: hostWin,
    location: { search: '?host=1' },
    URLSearchParams,
  });
  assert.equal(hostWin.ibisOfficeOptions, undefined);
});
