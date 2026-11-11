// SPDX-License-Identifier: AGPL-3.0-only
// Run against a local Web build. See docs/browser-html-export.md.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { chromium } from 'playwright-core';

const url = process.env.IBIS_HTML_TEST_URL ?? 'http://127.0.0.1:8794/editor.html?open=./samples/sorane-welcome.docx';
const executablePath = process.env.IBIS_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({ executablePath, headless: true, args: ['--enable-unsafe-webgpu', '--use-angle=swiftshader', '--disable-gpu-sandbox'] });
try {
  const page = await browser.newPage({ viewport: { width: 1024, height: 768 }, deviceScaleFactor: 1, acceptDownloads: true });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    if (message.type() === 'error' && /panicked|RuntimeError|RefCell already borrowed/.test(message.text())) errors.push(message.text());
  });
  await page.addInitScript(() => {
    localStorage.setItem('ibisoffice-lang', 'ja');
    window.__ibisBlobTypes = [];
    const create = URL.createObjectURL.bind(URL);
    URL.createObjectURL = blob => { window.__ibisBlobTypes.push(blob.type); return create(blob); };
  });
  await page.goto(url);
  await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await page.waitForTimeout(3000);
  await page.mouse.click(260, 278);
  await page.keyboard.insertText('HTML<&検証');
  await page.waitForTimeout(500);
  assert((await page.locator('textarea').inputValue()).includes('HTML<&検証'), 'test edit must reach the editor');

  await page.mouse.click(145, 49); // File
  await page.mouse.move(230, 266); // Export submenu
  await page.waitForTimeout(700);
  const downloadPromise = page.waitForEvent('download', { timeout: 15_000 });
  await page.mouse.click(520, 466); // Web page (.html)
  const download = await downloadPromise;
  assert.equal(download.suggestedFilename(), 'sorane-welcome.html');
  assert.equal(await download.failure(), null);
  const html = await readFile(await download.path(), 'utf8');
  assert(html.startsWith('<!DOCTYPE html>'), 'download must be HTML');
  assert(html.includes('HTML&lt;&amp;検証'), 'download must contain the escaped edit');
  assert(html.includes('Sorane Docs') && html.includes('<table>'), 'document text and table must be present');
  assert((await page.evaluate(() => window.__ibisBlobTypes)).includes('text/html;charset=utf-8'), 'HTML must use an HTML MIME type');
  assert.deepEqual(errors, []);
  console.log('PASS: browser downloads edited Word content as HTML');
} finally {
  await browser.close();
}
