// SPDX-License-Identifier: AGPL-3.0-only
// Run against a repository-root server after building www/; see docs/jtd-preview-proposal.md.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const fixture = fileURLToPath(new URL('../test/fixtures/jtd/synthetic.jtd', import.meta.url));
const original = await readFile(fixture);
const exportPath = join(await mkdtemp(join(tmpdir(), 'ibis-jtd-browser-')), 'export.docx');
const versionText = await readFile(new URL('../version.txt', import.meta.url), 'utf8');
const wasmVersion = versionText.match(/^wasm\s+(\S+)/m)?.[1];
assert(wasmVersion, 'version.txt must contain a wasm version');
const root = process.env.IBIS_JTD_TEST_ROOT ?? 'http://127.0.0.1:8798';
const executablePath = process.env.IBIS_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({
  executablePath,
  headless: true,
  args: ['--enable-unsafe-webgpu', '--use-angle=swiftshader', '--disable-gpu-sandbox'],
});
try {
  const page = await browser.newPage({ viewport: { width: 1024, height: 768 }, acceptDownloads: true });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    if (message.type() === 'error' && /panicked|RuntimeError/.test(message.text())) errors.push(message.text());
  });
  await page.addInitScript(version => {
    localStorage.setItem('ibisoffice-lang', 'ja');
    localStorage.setItem('ibisOffice.app.last-started-version', version);
    localStorage.setItem('ibisOffice.font-setup.checked-version', version);
    window.addEventListener('ibisoffice:download', event => { window.__download = Array.from(event.detail.bytes); });
  }, wasmVersion);
  await page.goto(`${root}/www/index.html?open=../test/fixtures/jtd/synthetic.jtd&download=1`);
  await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await page.waitForTimeout(3500);
  assert.match(await page.title(), /synthetic\.jtd/);
  await page.mouse.click(996, 18);
  await page.waitForFunction(() => !!window.__download);
  assert.deepEqual(Buffer.from(await page.evaluate(() => window.__download)), original);

  async function exportAt(y) {
    await page.mouse.click(135, 49);
    await page.mouse.move(225, 295);
    await page.waitForTimeout(500);
    const wait = page.waitForEvent('download', { timeout: 15_000 });
    await page.mouse.click(540, y);
    return wait;
  }
  let download = await exportAt(410);
  assert.equal(download.suggestedFilename(), 'synthetic.docx');
  await download.saveAs(exportPath);
  assert(execFileSync('unzip', ['-p', exportPath, 'word/document.xml'], { encoding: 'utf8' }).includes('日本語 test'));
  download = await exportAt(438);
  assert.equal(download.suggestedFilename(), 'synthetic.html');
  assert((await readFile(await download.path(), 'utf8')).includes('日本語 test'));

  // Reopen the page to leave the export menu in a known closed state.
  await page.reload();
  await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await page.waitForTimeout(3500);
  await page.mouse.click(135, 49);
  await page.waitForTimeout(600);
  const chosen = page.waitForEvent('filechooser', { timeout: 10_000 });
  await page.mouse.click(165, 110);
  await (await chosen).setFiles({ name: 'selected.jtd', mimeType: 'application/octet-stream', buffer: original });
  await page.waitForFunction(() => document.title.includes('selected.jtd'), null, { timeout: 60_000 });
  await page.waitForTimeout(1000);
  await page.evaluate(() => { window.__download = null; });
  await page.mouse.click(996, 18);
  await page.waitForFunction(() => !!window.__download);
  assert.deepEqual(Buffer.from(await page.evaluate(() => window.__download)), original);
  assert.deepEqual(errors, []);
  console.log('PASS initial JTD, original download, DOCX and HTML export, file picker; no browser panic');
} finally {
  await browser.close();
}
