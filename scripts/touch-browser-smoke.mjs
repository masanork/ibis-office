// SPDX-License-Identifier: AGPL-3.0-only
// Run against a local build served at IBIS_TOUCH_URL (see docs/touch-browser-test.md).
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';

const url = process.env.IBIS_TOUCH_URL ?? 'http://127.0.0.1:8793/editor.html?open=./samples/sorane-welcome.docx';
const executablePath = process.env.IBIS_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({
  executablePath,
  headless: true,
  args: ['--enable-unsafe-webgpu', '--use-angle=swiftshader', '--disable-gpu-sandbox'],
});
try {
  const page = await browser.newPage({ viewport: { width: 1024, height: 768 }, deviceScaleFactor: 1, hasTouch: true, isMobile: true });
  const failures = [];
  page.on('pageerror', error => failures.push(error.message));
  page.on('console', message => {
    if (message.type() === 'error' && /panicked|RuntimeError|RefCell already borrowed/.test(message.text())) failures.push(message.text());
  });
  await page.addInitScript(() => {
    window.__ibisTouchEvents = [];
    for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) {
      document.addEventListener(type, event => {
        if (event.pointerType === 'touch') window.__ibisTouchEvents.push(type);
      }, true);
    }
  });
  await page.goto(url);
  await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await page.waitForTimeout(3000);
  const cdp = await page.context().newCDPSession(page);
  const pt = (id, x, y) => ({ id, x, y, radiusX: 4, radiusY: 4, force: 1 });
  const touch = (type, touchPoints) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  const zoomLabel = () => page.screenshot({ clip: { x: 937, y: 742, width: 50, height: 26 } });
  const workspace = () => page.screenshot({ clip: { x: 40, y: 150, width: 800, height: 500 } });
  const settle = () => page.waitForTimeout(1000);

  const initialZoom = await zoomLabel();
  await touch('touchStart', [pt(1, 450, 400)]);
  await touch('touchStart', [pt(1, 450, 400), pt(2, 550, 400)]);
  await touch('touchMove', [pt(1, 400, 400), pt(2, 600, 400)]);
  await touch('touchEnd', [pt(1, 400, 400)]);
  await touch('touchEnd', []);
  await settle();
  const zoomed = await zoomLabel();
  assert(!initialZoom.equals(zoomed), 'pinch out must change the displayed zoom');

  await touch('touchStart', [pt(3, 400, 400)]);
  await touch('touchStart', [pt(3, 400, 400), pt(4, 600, 400)]);
  await touch('touchMove', [pt(3, 450, 400), pt(4, 550, 400)]);
  await touch('touchEnd', [pt(3, 450, 400)]);
  await touch('touchEnd', []);
  await settle();
  assert(initialZoom.equals(await zoomLabel()), 'inverse pinch must restore the displayed zoom');

  await touch('touchStart', [pt(5, 916, 755)]);
  await touch('touchEnd', []);
  await settle();
  assert(!initialZoom.equals(await zoomLabel()), 'one-finger tap must activate the zoom button');

  const beforeScroll = await workspace();
  await touch('touchStart', [pt(6, 750, 600)]);
  await touch('touchMove', [pt(6, 750, 350)]);
  await touch('touchEnd', []);
  await settle();
  assert(!beforeScroll.equals(await workspace()), 'one-finger pan must scroll the document');
  const events = await page.evaluate(() => window.__ibisTouchEvents);
  assert(events.includes('pointerdown') && events.includes('pointermove') && events.includes('pointerup'), 'browser must dispatch touch pointer events');
  assert.deepEqual(failures, [], 'touch interactions must not panic');
  console.log('PASS: browser touch tap, pinch in/out, and single-finger scroll');
} finally {
  await browser.close();
}
