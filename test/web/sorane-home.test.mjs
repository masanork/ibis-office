// SPDX-License-Identifier: AGPL-3.0-only
import { readFileSync, existsSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';

const home = readFileSync(new URL('../../www/index.html', import.meta.url), 'utf8');
const editor = readFileSync(new URL('../../www/editor.html', import.meta.url), 'utf8');
const startScript = readFileSync(new URL('../../www/sorane-start.js', import.meta.url), 'utf8');

test('index.html boots the WASM app directly with start-view customization', () => {
  assert.match(home, /<script src="\.\/boot\.js"><\/script>/);
  assert.match(home, /<script src="\.\/sorane-start\.js"><\/script>/);
  assert.match(home, /<script type="module" src="\.\/main\.js"><\/script>/);
  assert.match(home, /<script type="module" src="\.\/sorane-ai\.js"><\/script>/);
  assert.match(editor, /<script src="\.\/sorane-start\.js"><\/script>/);
});

test('sorane-start.js configures the six featured samples and all bundled files exist', () => {
  const win = {};
  runInNewContext(startScript, {
    window: win,
    location: { search: '' },
    URLSearchParams,
    fetch: () => Promise.resolve({ ok: false }),
  });
  const startView = win.ibisOfficeOptions?.startView;
  assert.ok(startView, 'startView config should be installed');
  assert.equal(startView.items.length, 13);
  for (const name of [
    'mermaid-showcase.md',
    'mynumber-law-2023-comparison.jtd',
    'data-action-plan-2023.pptx',
    'urban-comfort-analysis.xlsx',
    'standard-linkage.csv',
    'visio-multi-page-handbook.vsdx',
    'hancom-sample.hwpx',
    'ofd-sample.ofd',
    'odf-charts-sample.ods',
    'ibis-office-wasm.sbom.cdx.json',
    'ibis-office-wasm.cbom.cdx.json',
    'sbom-vulnerability-review.xlsx',
  ]) {
    assert.ok(startView.items.some((item) => item.url === `/samples/${name}`), `missing sample ${name}`);
    assert.ok(existsSync(new URL(`../../www/samples/${name}`, import.meta.url)), `file missing: ${name}`);
  }
  assert.ok(
    startView.items.some((item) => item.url === '/api/samples/bddbceda-c82b-4730-8744-a50e6ccb4600/sample.xlsx'),
  );
  assert.ok(startView.panel?.links?.some((l) => l.url === '/files'));
  assert.ok(startView.panel?.links?.some((l) => l.url === '/repos'));
});

test('sorane-start.js stays out of the way in host mode and merges extra /api/samples', async () => {
  const hostWin = {};
  runInNewContext(startScript, {
    window: hostWin,
    location: { search: '?host=1' },
    URLSearchParams,
    fetch: () => Promise.resolve({ ok: false }),
  });
  assert.equal(hostWin.ibisOfficeOptions, undefined);

  let updated = null;
  const win = {
    ibisOfficeSetStartView: (cfg) => { updated = cfg; },
  };
  runInNewContext(startScript, {
    window: win,
    location: { search: '' },
    URLSearchParams,
    fetch: () => Promise.resolve({
      ok: true,
      json: () => Promise.resolve([
        { id: 'bddbceda-c82b-4730-8744-a50e6ccb4600', name: 'sample.xlsx', title: 'duplicate featured' },
        { id: '11111111-2222-3333-4444-555555555555', name: 'extra.docx', title: '追加サンプル' },
      ]),
    }),
  });
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.ok(updated);
  assert.equal(updated.items.length, 14);
  assert.equal(updated.items[13].title, '追加サンプル');
  assert.equal(updated.items[13].url, '/api/samples/11111111-2222-3333-4444-555555555555/extra.docx');
});
