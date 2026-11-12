// SPDX-License-Identifier: AGPL-3.0-only
// Run against the pinned upstream GPUI /input example built as described in
// docs/gpui-upstream-touch-audit.md. Chrome CDP touch is synthetic; this test
// observes the IME textarea's inputmode, not a physical software keyboard.
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';

const url = process.env.GPUI_UPSTREAM_WEB_URL ?? 'http://127.0.0.1:8799/';
const executablePath = process.env.IBIS_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({
  executablePath,
  headless: true,
  args: ['--disable-webgpu', '--use-angle=swiftshader', '--disable-gpu-sandbox'],
});
try {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  const page = await context.newPage();
  const failures = [];
  page.on('pageerror', error => failures.push(error.message));
  await page.addInitScript(() => {
    window.__auditTouchTrace = [];
    for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) {
      document.addEventListener(type, event => {
        if (event.pointerType === 'touch') window.__auditTouchTrace.push({ type, id: event.pointerId });
      }, true);
    }
  });
  await page.goto(url);
  await page.locator('canvas').waitFor({ timeout: 120_000 });
  await page.waitForTimeout(3000);
  const mirror = page.locator('textarea').first();
  // The example initially focuses the editor and enables the keyboard.
  // Set a controlled dismissed state to isolate the next gesture.
  await mirror.evaluate(element => element.setAttribute('inputmode', 'none'));
  assert.equal(await mirror.getAttribute('inputmode'), 'none');
  const cdp = await context.newCDPSession(page);
  const point = (id, x, y) => ({ id, x, y, radiusX: 4, radiusY: 4, force: 1 });
  const touch = (type, touchPoints) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  await touch('touchStart', [point(1, 65, 45)]);
  await touch('touchStart', [point(1, 65, 45), point(2, 165, 45)]);
  await touch('touchMove', [point(1, 65, 45), point(2, 220, 45)]);
  await touch('touchEnd', [point(2, 220, 45)]);
  await touch('touchEnd', []);
  await page.waitForTimeout(500);
  const trace = await page.evaluate(() => window.__auditTouchTrace);
  assert.equal(trace.filter(event => event.type === 'pointerdown').length, 2);
  assert.equal(trace.filter(event => event.type === 'pointerup').length, 2);
  assert(trace.some(event => event.type === 'pointermove'), 'one contact must move');
  assert.equal(await mirror.getAttribute('inputmode'), 'text', 'pinned upstream example re-enables the virtual keyboard');
  assert.deepEqual(failures, []);
  console.log('Reproduced: two-contact gesture changes inputmode from none to text');
  console.log(JSON.stringify(trace));
  await context.close();
} finally {
  await browser.close();
}
