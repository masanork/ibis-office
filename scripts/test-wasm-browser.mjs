// SPDX-License-Identifier: AGPL-3.0-only
// Run after building and serving www/. Install playwright-core separately.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { chromium } from 'playwright-core';
const mode = process.argv[2];
assert(['split', 'standard'].includes(mode), 'pass split or standard');
const version = (await readFile(new URL('../version.txt', import.meta.url), 'utf8')).match(/^wasm\s+(\S+)/m)?.[1];
assert(version, 'version.txt must contain a wasm version');
const url = process.env.IBIS_WEB_TEST_URL ?? 'http://127.0.0.1:8799/www/index.html';
const executablePath = process.env.IBIS_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({ executablePath, headless: true, args: ['--enable-unsafe-webgpu', '--use-angle=swiftshader', '--disable-gpu-sandbox'] });
try {
  const page = await browser.newPage({ viewport: { width: 1024, height: 768 } });
  const errors = [], requests = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error' && /panic|RuntimeError/.test(message.text())) errors.push(message.text()); });
  page.on('request', request => requests.push(request.url()));
  await page.addInitScript(value => {
    localStorage.setItem('ibisOffice.app.last-started-version', value);
    localStorage.setItem('ibisOffice.font-setup.checked-version', value);
  }, version);
  await page.goto(url);
  await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await page.waitForTimeout(2000);
  assert.equal(await page.locator('canvas').count(), 1);
  assert.deepEqual(errors, []);
  const chunks = requests.filter(request => request.endsWith('.chunk')).length;
  const regular = requests.some(request => request.endsWith('ibis_office_app_bg.wasm'));
  if (mode === 'split') { assert(chunks > 0); assert(!regular); }
  else { assert.equal(chunks, 0); assert(regular); }
  console.log(`PASS ${mode}: editor opened, ${chunks} chunks, no browser panic`);
} finally {
  await browser.close();
}
