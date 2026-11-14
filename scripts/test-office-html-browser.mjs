// SPDX-License-Identifier: AGPL-3.0-only
// Run against a local repository-root server. See docs/browser-html-export.md.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { chromium } from 'playwright-core';
import { wasmVersion } from './browser-test-version.mjs';

const root = process.env.IBIS_HTML_TEST_ROOT ?? 'http://127.0.0.1:8800';
const executablePath = process.env.IBIS_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
for (const ext of ['xlsx', 'pptx']) {
  const browser = await chromium.launch({ executablePath, headless: true, args: ['--enable-unsafe-webgpu', '--use-angle=swiftshader', '--disable-gpu-sandbox'] });
  try {
    const errors = [];
    const page = await browser.newPage({ viewport: { width: 1024, height: 768 }, deviceScaleFactor: 1, acceptDownloads: true });
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error' && /panicked|RuntimeError|RefCell already borrowed/.test(message.text())) errors.push(message.text()); });
    await page.addInitScript(version => {
      localStorage.setItem('ibisoffice-lang', 'ja');
      localStorage.setItem('ibisOffice.app.last-started-version', version);
      localStorage.setItem('ibisOffice.font-setup.checked-version', version);
    }, wasmVersion);
    await page.goto(`${root}/www/index.html?open=../test/web/fixtures/html-export.${ext}`);
    await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
    await page.waitForTimeout(3000);
    if (ext === 'xlsx') {
      await page.mouse.dblclick(70, 165);
      await page.keyboard.insertText('Excel編集');
      await page.keyboard.press('Enter');
    } else {
      await page.mouse.dblclick(400, 196);
      await page.keyboard.insertText('Slide編集');
    }
    await page.waitForTimeout(500);
    await page.mouse.click(145, 49);
    await page.mouse.move(230, ext === 'xlsx' ? 323 : 295);
    await page.waitForTimeout(700);
    const downloadPromise = page.waitForEvent('download', { timeout: 15_000 });
    await page.mouse.click(520, ext === 'xlsx' ? 635 : 494);
    const download = await downloadPromise;
    assert.equal(download.suggestedFilename(), 'html-export.html');
    assert.equal(await download.failure(), null);
    const html = await readFile(await download.path(), 'utf8');
    assert(html.startsWith('<!DOCTYPE html>'));
    if (ext === 'xlsx') {
      assert(html.includes('Excel編集') && html.includes('Second sheet'));
      assert(html.includes('colspan="2"') && html.includes('123.5'));
      assert(!html.includes('hidden sheet secret') && !html.includes('hidden row secret') && !html.includes('hidden column secret'));
    } else {
      assert(html.includes('Slide編集') && html.includes('Third slide'));
      assert(html.includes('data:image/png;base64,') && html.includes('Cell A') && html.includes('Cell B'));
      assert(!html.includes('hidden slide secret'));
    }
    assert.deepEqual(errors, []);
    await page.context().close();
    const preview = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 1, isMobile: true, hasTouch: true });
    await preview.setContent(html);
    assert((await preview.locator('body').innerText()).includes(ext === 'xlsx' ? 'Excel編集' : 'Slide編集'));
    if (ext === 'pptx') {
      assert.equal(await preview.locator('.slide').count(), 2);
      await preview.locator('img').first().evaluate(image => image.decode());
      assert(await preview.locator('img').first().evaluate(image => image.naturalWidth > 0));
    } else {
      assert.equal(await preview.locator('table').count(), 2);
    }
    if (process.env.IBIS_HTML_ARTIFACTS) await preview.screenshot({ path: `${process.env.IBIS_HTML_ARTIFACTS}/${ext}-output.png`, fullPage: true });
    await preview.context().close();
    console.log(`PASS: ${ext} edited HTML download and mobile rendering`);
  } finally { await browser.close(); }
}
