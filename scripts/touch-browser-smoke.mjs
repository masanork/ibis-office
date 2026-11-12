// SPDX-License-Identifier: AGPL-3.0-only
// Run against a local build served at IBIS_TOUCH_URL (see docs/touch-browser-test.md).
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chromium } from 'playwright-core';

const url = process.env.IBIS_TOUCH_URL ?? 'http://127.0.0.1:8793/editor.html?open=./samples/sorane-welcome.docx';
const executablePath = process.env.IBIS_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({
  executablePath,
  headless: true,
  args: [
    ...(process.env.IBIS_TOUCH_WEBGL === '1' ? ['--disable-webgpu'] : ['--enable-unsafe-webgpu']),
    '--use-angle=swiftshader', '--disable-gpu-sandbox',
  ],
});
// `run()` can return before the first GPU frame is presented. Wait for
// actual pixels rather than assuming that removal of the loading overlay
// means the editor is already ready for input.
async function waitForEditorFrame(page) {
  console.log(`Waiting for editor frame at ${page.viewportSize().width}px`);
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    const screenshot = await page.screenshot();
    const painted = await page.evaluate(async data => {
      const img = new Image();
      img.src = `data:image/png;base64,${data}`;
      await img.decode();
      const canvas = document.createElement('canvas');
      canvas.width = img.width;
      canvas.height = img.height;
      const ctx = canvas.getContext('2d');
      ctx.drawImage(img, 0, 0);
      const pixels = ctx.getImageData(0, 0, img.width, img.height).data;
      let nonwhite = 0;
      for (let i = 0; i < pixels.length; i += 64) {
        if (pixels[i] < 230 || pixels[i + 1] < 230 || pixels[i + 2] < 230) nonwhite++;
      }
      return nonwhite > 100;
    }, screenshot.toString('base64'));
    if (painted) { console.log('Editor frame painted'); return; }
    await page.waitForTimeout(500);
  }
  if (process.env.IBIS_TOUCH_ARTIFACTS) await page.screenshot({ path: `${process.env.IBIS_TOUCH_ARTIFACTS}/no-editor-frame.png` });
  throw new Error('editor did not paint a frame within 60 seconds');
}
try {
  const page = await browser.newPage({ viewport: { width: 1024, height: 768 }, deviceScaleFactor: 1, hasTouch: true, isMobile: true });
  const failures = [];
  page.on('pageerror', error => failures.push(error.message));
  page.on('console', message => {
    if (message.type() === 'error') console.error('Browser console:', message.text());
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
  if (process.env.IBIS_TOUCH_WEBGL === '1') {
    assert.equal(await page.evaluate(async () => !!(await navigator.gpu?.requestAdapter())), false, 'WebGL test must not have a WebGPU adapter');
  }
  await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await waitForEditorFrame(page);
  await page.waitForTimeout(1000);
  const cdp = await page.context().newCDPSession(page);
  const pt = (id, x, y) => ({ id, x, y, radiusX: 4, radiusY: 4, force: 1 });
  const touch = (type, touchPoints) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  const zoomLabel = () => page.screenshot({ clip: { x: 937, y: 742, width: 50, height: 26 } });
  const workspace = () => page.screenshot({ clip: { x: 40, y: 150, width: 800, height: 500 } });
  const settle = () => page.waitForTimeout(1000);
  const waitForZoom = async (baseline, equal, message) => {
    const deadline = Date.now() + 30_000;
    do {
      if ((await zoomLabel()).equals(baseline) === equal) return;
      await page.waitForTimeout(250);
    } while (Date.now() < deadline);
    if (process.env.IBIS_TOUCH_ARTIFACTS) await page.screenshot({ path: `${process.env.IBIS_TOUCH_ARTIFACTS}/zoom-failure.png` });
    assert.fail(message);
  };

  const initialZoom = await zoomLabel();
  if (process.env.IBIS_TOUCH_ARTIFACTS) await page.screenshot({ path: `${process.env.IBIS_TOUCH_ARTIFACTS}/pinch-before.png` });
  await touch('touchStart', [pt(1, 450, 400)]);
  await touch('touchStart', [pt(1, 450, 400), pt(2, 550, 400)]);
  await touch('touchMove', [pt(1, 400, 400), pt(2, 600, 400)]);
  await touch('touchEnd', [pt(1, 400, 400)]);
  await touch('touchEnd', []);
  await waitForZoom(initialZoom, false, 'pinch out must change the displayed zoom');
  const zoomed = await zoomLabel();
  if (process.env.IBIS_TOUCH_ARTIFACTS) await page.screenshot({ path: `${process.env.IBIS_TOUCH_ARTIFACTS}/pinch-after.png` });
  assert(!initialZoom.equals(zoomed), 'pinch out must change the displayed zoom');

  await touch('touchStart', [pt(3, 400, 400)]);
  await touch('touchStart', [pt(3, 400, 400), pt(4, 600, 400)]);
  await touch('touchMove', [pt(3, 450, 400), pt(4, 550, 400)]);
  await touch('touchEnd', [pt(3, 450, 400)]);
  await touch('touchEnd', []);
  await waitForZoom(initialZoom, true, 'inverse pinch must restore the displayed zoom');

  await touch('touchStart', [pt(5, 916, 755)]);
  await touch('touchEnd', []);
  await waitForZoom(initialZoom, false, 'one-finger tap must activate the zoom button');

  const beforeScroll = await workspace();
  await touch('touchStart', [pt(6, 750, 600)]);
  await touch('touchMove', [pt(6, 750, 350)]);
  await touch('touchEnd', []);
  await settle();
  assert(!beforeScroll.equals(await workspace()), 'one-finger pan must scroll the document');
  const events = await page.evaluate(() => window.__ibisTouchEvents);
  assert(events.includes('pointerdown') && events.includes('pointermove') && events.includes('pointerup'), 'browser must dispatch touch pointer events');
  assert.deepEqual(failures, [], 'touch interactions must not panic');

  await page.context().close();
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 1, hasTouch: true, isMobile: true });
  phone.on('pageerror', error => failures.push(error.message));
  phone.on('console', message => {
    if (message.type() === 'error' && /panicked|RuntimeError|RefCell already borrowed/.test(message.text())) failures.push(message.text());
  });
  await phone.goto(url);
  await phone.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await waitForEditorFrame(phone);
  await phone.waitForTimeout(1000);
  const pixel = x => phone.screenshot({ clip: { x, y: 180, width: 1, height: 1 } });
  assert((await pixel(25)).equals(await pixel(385)), 'phone document should fit between the page margins');
  const phoneCdp = await phone.context().newCDPSession(phone);
  const phoneTouch = (type, touchPoints) => phoneCdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  const phoneZoom = () => phone.screenshot({ clip: { x: 303, y: 808, width: 58, height: 36 } });
  const beforeTap = await phoneZoom();
  await phoneTouch('touchStart', [pt(7, 277, 826)]);
  await phoneTouch('touchEnd', []);
  await phone.waitForTimeout(1000);
  assert(!beforeTap.equals(await phoneZoom()), 'phone zoom-in button must be visible and tappable');
  const documentView = () => phone.screenshot({ clip: { x: 50, y: 170, width: 290, height: 520 } });
  const pan = async (id, startY, endY) => {
    await phoneTouch('touchStart', [pt(id, 200, startY)]);
    for (let step = 1; step <= 10; step++) {
      await phoneTouch('touchMove', [pt(id, 200, startY + (endY - startY) * step / 10)]);
      await phone.waitForTimeout(60);
    }
    await phone.waitForTimeout(100); // release at rest, without a fling
    await phoneTouch('touchEnd', []);
    await phone.waitForTimeout(500);
  };
  for (let i = 0; i < 5; i++) {
    await phoneTouch('touchStart', [pt(30 + i, 277, 826)]);
    await phoneTouch('touchEnd', []);
    await phone.waitForTimeout(150);
  }
  await phone.waitForTimeout(500);
  const startView = await documentView();
  await pan(20, 600, 420);
  const downView = await documentView();
  await pan(21, 420, 600);
  const restoredView = await documentView();
  // Look for translated document pixels, not merely different screenshots:
  // a blinking caret or other animation can change an image without scrolling.
  const displacement = async (before, after, direction) => phone.evaluate(async ({ before, after, direction }) => {
    const pixels = async data => {
      const img = new Image();
      img.src = `data:image/png;base64,${data}`;
      await img.decode();
      const canvas = document.createElement('canvas');
      canvas.width = img.width;
      canvas.height = img.height;
      const context = canvas.getContext('2d');
      context.drawImage(img, 0, 0);
      const rgba = context.getImageData(0, 0, img.width, img.height).data;
      return { width: img.width, height: img.height, dark: Uint8Array.from({ length: img.width * img.height }, (_, i) => rgba[i * 4] < 170 && rgba[i * 4 + 1] < 170 && rgba[i * 4 + 2] < 170 ? 1 : 0) };
    };
    const a = await pixels(before);
    const b = await pixels(after);
    let best = { shift: 0, score: 0, foreground: 0 };
    for (let magnitude = 130; magnitude <= 220; magnitude++) {
      const shift = direction * magnitude;
      let intersection = 0, union = 0;
      for (let y = Math.max(0, -shift); y < Math.min(a.height, a.height - shift); y++) {
        for (let x = 0; x < a.width; x++) {
          const first = a.dark[y * a.width + x];
          const second = b.dark[(y + shift) * a.width + x];
          intersection += first & second;
          union += first | second;
        }
      }
      const score = union ? intersection / union : 0;
      if (score > best.score) best = { shift, score, foreground: union };
    }
    return best;
  }, { before: before.toString('base64'), after: after.toString('base64'), direction });
  if (process.env.IBIS_TOUCH_ARTIFACTS) {
    const { writeFile } = await import('node:fs/promises');
    await writeFile(`${process.env.IBIS_TOUCH_ARTIFACTS}/scroll-start.png`, startView);
    await writeFile(`${process.env.IBIS_TOUCH_ARTIFACTS}/scroll-down.png`, downView);
    await writeFile(`${process.env.IBIS_TOUCH_ARTIFACTS}/scroll-restored.png`, restoredView);
  }
  const forward = await displacement(startView, downView, -1);
  const reverse = await displacement(downView, restoredView, 1);
  const stationary = await displacement(startView, startView, -1);
  assert(forward.score > 0.5 && forward.foreground > 1000, `phone swipe up must move document pixels upward: ${JSON.stringify(forward)}`);
  assert(reverse.score > 0.5 && reverse.foreground > 1000, `phone swipe down must move document pixels downward: ${JSON.stringify(reverse)}`);
  assert(stationary.score < 0.5, `stationary document pixels must not count as a scroll: ${JSON.stringify(stationary)}`);
  const menuStrip = () => phone.screenshot({ clip: { x: 0, y: 34, width: 390, height: 30 } });
  const beforeMenu = await menuStrip();
  if (process.env.IBIS_TOUCH_ARTIFACTS) await phone.screenshot({ path: `${process.env.IBIS_TOUCH_ARTIFACTS}/phone-before.png` });
  await phoneTouch('touchStart', [pt(8, 350, 49)]);
  await phoneTouch('touchMove', [pt(8, 80, 49)]);
  await phoneTouch('touchEnd', []);
  await phone.waitForTimeout(1000);
  assert(!beforeMenu.equals(await menuStrip()), 'phone menu bar must scroll horizontally');
  if (process.env.IBIS_TOUCH_ARTIFACTS) await phone.screenshot({ path: `${process.env.IBIS_TOUCH_ARTIFACTS}/phone-after.png` });
  const menuPopup = () => phone.screenshot({ clip: { x: 150, y: 66, width: 230, height: 300 } });
  const beforePopup = await menuPopup();
  await phoneTouch('touchStart', [pt(9, 360, 49)]);
  await phoneTouch('touchEnd', []);
  await phone.waitForTimeout(1000);
  assert(!beforePopup.equals(await menuPopup()), 'a revealed menu must open on tap');
  assert.deepEqual(failures, [], 'phone interactions must not panic');
  await phone.context().close();

  // Open from the file picker, whose source is retained for the download action.
  const editing = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 1, hasTouch: true, isMobile: true });
  editing.on('pageerror', error => failures.push(error.message));
  editing.on('console', message => {
    if (message.type() === 'error' && /panicked|RuntimeError|RefCell already borrowed/.test(message.text())) failures.push(message.text());
  });
  await editing.addInitScript(() => window.addEventListener('ibisoffice:download', event => { window.__ibisDownloaded = Array.from(event.detail.bytes); }));
  const editingUrl = new URL(url);
  editingUrl.searchParams.set('download', '1');
  await editing.goto(editingUrl.href);
  await editing.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await waitForEditorFrame(editing);
  await editing.waitForTimeout(1000);
  const editingCdp = await editing.context().newCDPSession(editing);
  const editingTouch = (type, touchPoints) => editingCdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  const editingTap = async (id, x, y) => {
    await editingTouch('touchStart', [pt(id, x, y)]);
    await editingTouch('touchEnd', []);
  };
  const assertDownloadContains = async marker => {
    await editing.waitForFunction(() => Array.isArray(window.__ibisDownloaded), null, { timeout: 15_000 });
    const docx = Buffer.from(await editing.evaluate(() => window.__ibisDownloaded));
    execFileSync('python3', ['-c', "import io,sys,zipfile; z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read())); assert sys.argv[1] in z.read('word/document.xml').decode('utf-8')", marker], { input: docx });
    await editing.evaluate(() => { delete window.__ibisDownloaded; });
  };
  await editingTap(15, 120, 200);
  await editing.waitForTimeout(500);
  await editing.keyboard.insertText('初期編集');
  await editing.waitForTimeout(500);
  await editingTap(16, 362, 18); // Download the initially opened document.
  await assertDownloadContains('初期編集');

  await editingTap(10, 43, 49); // File
  await editing.waitForTimeout(300);
  const chooser = editing.waitForEvent('filechooser', { timeout: 15_000 });
  await editingTap(11, 60, 110); // Open
  await (await chooser).setFiles('www/samples/sorane-welcome.docx');
  await editing.waitForTimeout(3000);
  await editingTouch('touchStart', [pt(12, 300, 550)]);
  await editingTouch('touchMove', [pt(12, 300, 400)]);
  await editingTouch('touchEnd', []);
  await editing.waitForTimeout(500);
  const mirror = editing.locator('textarea');
  assert.equal(await mirror.getAttribute('inputmode'), 'none', 'a pan must not summon the virtual keyboard');
  await editingTap(13, 120, 200);
  await editing.waitForTimeout(500);
  assert.equal(await mirror.getAttribute('inputmode'), 'text', 'an editable tap must enable the virtual keyboard');
  assert(await mirror.evaluate(element => document.activeElement === element), 'the IME input must be focused');
  await editing.keyboard.insertText('Touch検証');
  await editing.waitForTimeout(500);
  assert((await mirror.inputValue()).includes('Touch検証'), 'the input mirror must receive inserted text');
  await editingTap(14, 362, 18); // Embed download
  await assertDownloadContains('Touch検証');
  assert.deepEqual(failures, [], 'editing interactions must not panic');
  console.log('PASS: touch viewing, phone controls, editable tap, and DOCX download');
} finally {
  await browser.close();
}
