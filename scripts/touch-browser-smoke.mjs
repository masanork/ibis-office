// SPDX-License-Identifier: AGPL-3.0-only
// Run against a local build served at IBIS_TOUCH_URL (see docs/touch-browser-test.md).
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chromium } from 'playwright-core';
import { wasmVersion } from './browser-test-version.mjs';

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
  await page.addInitScript(version => {
    Error.stackTraceLimit = 100;
    localStorage.setItem('ibisOffice.app.last-started-version', version);
    localStorage.setItem('ibisOffice.font-setup.checked-version', version);
    window.__ibisTouchEvents = [];
    window.__ibisTouchTrace = [];
    for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) {
      document.addEventListener(type, event => {
        if (event.pointerType === 'touch') {
          window.__ibisTouchEvents.push(type);
          window.__ibisTouchTrace.push({ type, id: event.pointerId, x: event.clientX, y: event.clientY, time: event.timeStamp });
        }
      }, true);
    }
  }, wasmVersion);
  await page.goto(url);
  if (process.env.IBIS_TOUCH_WEBGL === '1') {
    assert.equal(await page.evaluate(async () => !!(await navigator.gpu?.requestAdapter())), false, 'WebGL test must not have a WebGPU adapter');
  }
  await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await waitForEditorFrame(page);
  await page.waitForTimeout(1000);
  const cdp = await page.context().newCDPSession(page);
  if (process.env.IBIS_TOUCH_CPU_THROTTLE) await cdp.send('Emulation.setCPUThrottlingRate', { rate: Number(process.env.IBIS_TOUCH_CPU_THROTTLE) });
  const pt = (id, x, y) => ({ id, x, y, radiusX: 4, radiusY: 4, force: 1 });
  const touch = (type, touchPoints) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  // Queue both commands together. Awaiting the start acknowledgement first
  // can turn a tap into a long press on a slow software renderer.
  const tap = async (send, id, x, y) => {
    await Promise.all([send('touchStart', [pt(id, x, y)]), send('touchEnd', [])]);
  };
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
    console.error('Touch event trace:', JSON.stringify(await page.evaluate(() => window.__ibisTouchTrace)));
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

  await tap(touch, 5, 916, 755);
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
  await phone.addInitScript(version => {
    localStorage.setItem('ibisOffice.app.last-started-version', version);
    localStorage.setItem('ibisOffice.font-setup.checked-version', version);
  }, wasmVersion);
  const phoneEditorUrl = new URL(url);
  phoneEditorUrl.searchParams.set('view', '0');
  await phone.goto(phoneEditorUrl.href);
  await phone.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await waitForEditorFrame(phone);
  await phone.waitForTimeout(1000);
  const pixel = x => phone.screenshot({ clip: { x, y: 180, width: 1, height: 1 } });
  assert((await pixel(25)).equals(await pixel(385)), 'phone document should fit between the page margins');
  const phoneCdp = await phone.context().newCDPSession(phone);
  const phoneTouch = (type, touchPoints) => phoneCdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  const phoneZoom = () => phone.screenshot({ clip: { x: 303, y: 808, width: 58, height: 36 } });
  const beforeTap = await phoneZoom();
  await tap(phoneTouch, 7, 277, 826);
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
    await tap(phoneTouch, 30 + i, 277, 826);
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
  // The menu overflow is available after switching the phone to editing.
  await tap(phoneTouch, 8, 350, 24);
  await phone.waitForTimeout(1000);
  const menuPopup = () => phone.screenshot({ clip: { x: 150, y: 66, width: 230, height: 300 } });
  const beforePopup = await menuPopup();
  if (process.env.IBIS_TOUCH_ARTIFACTS) await phone.screenshot({ path: `${process.env.IBIS_TOUCH_ARTIFACTS}/phone-before.png` });
  // The English system font on Linux makes the preceding menu labels narrower.
  const overflowX = process.platform === 'linux' ? 331 : 354;
  await tap(phoneTouch, 9, overflowX, 61);
  await phone.waitForTimeout(1000);
  assert(!beforePopup.equals(await menuPopup()), 'the overflow menu must open on touch');
  if (process.env.IBIS_TOUCH_ARTIFACTS) await phone.screenshot({ path: `${process.env.IBIS_TOUCH_ARTIFACTS}/phone-after.png` });
  assert.deepEqual(failures, [], 'phone interactions must not panic');
  await phone.context().close();

  // A standalone phone opens files in viewing mode and can switch to editing.
  const viewer = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 1, hasTouch: true, isMobile: true });
  viewer.on('pageerror', error => failures.push(error.message));
  await viewer.addInitScript(version => {
    localStorage.setItem('ibisOffice.app.last-started-version', version);
    localStorage.setItem('ibisOffice.font-setup.checked-version', version);
  }, wasmVersion);
  await viewer.goto(url);
  await viewer.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await waitForEditorFrame(viewer);
  await viewer.waitForTimeout(1000);
  const viewerCdp = await viewer.context().newCDPSession(viewer);
  const viewerTouch = (type, touchPoints) => viewerCdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  const viewerMirror = viewer.locator('textarea');
  await tap(viewerTouch, 99, 120, 150);
  assert.equal(await viewerMirror.getAttribute('inputmode'), 'none', 'viewing must not open the virtual keyboard');
  const viewerChooser = viewer.waitForEvent('filechooser', { timeout: 15_000 });
  await tap(viewerTouch, 100, 260, 24);
  await (await viewerChooser).setFiles('www/samples/sorane-welcome.docx');
  await viewer.waitForTimeout(1000);

  await tap(viewerTouch, 101, 350, 24); // Edit
  await viewer.waitForTimeout(500);
  await tap(viewerTouch, 102, 120, 200);
  await viewer.waitForFunction(() => document.querySelector('textarea')?.inputMode === 'text');
  await viewer.keyboard.insertText('Mode switch test');
  await viewer.waitForFunction(() => document.querySelector('textarea')?.value.includes('Mode switch test'));

  await tap(viewerTouch, 103, 350, 24); // Viewing
  await viewer.waitForFunction(() => document.querySelector('textarea')?.inputMode === 'none');
  await tap(viewerTouch, 104, 120, 150);
  assert.equal(await viewerMirror.getAttribute('inputmode'), 'none', 'returning to viewing must suppress the keyboard');

  await tap(viewerTouch, 105, 350, 24); // Edit again
  await viewer.waitForTimeout(500);
  await tap(viewerTouch, 106, 120, 200);
  await viewer.waitForFunction(() => document.querySelector('textarea')?.inputMode === 'text');
  assert((await viewerMirror.inputValue()).includes('Mode switch test'), 'edits must survive a round trip through viewing mode');
  assert.deepEqual(failures, [], 'switching phone modes must not panic');
  await viewer.context().close();

  // A fixed-layout file must render its page, including both sides on a phone.
  // Generate the fixtures here so the browser test needs no local documents.
  const stream = '0 0.8 0 rg 12 642 588 120 re f';
  const pdfObjects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>',
    `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
  ];
  let pdf = '%PDF-1.4\n';
  const offsets = pdfObjects.map((object, index) => {
    const offset = pdf.length;
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
    return String(offset).padStart(10, '0');
  });
  pdf += `xref\n0 5\n0000000000 65535 f \n${offsets.map(o => `${o} 00000 n \n`).join('')}trailer\n<< /Root 1 0 R /Size 5 >>\nstartxref\n${pdf.length}\n%%EOF\n`;
  const ofd = execFileSync('python3', ['-c', `
import io, sys, zipfile
buf = io.BytesIO()
ns = 'xmlns="http://www.ofdspec.org/2016"'
with zipfile.ZipFile(buf, 'w') as z:
    z.writestr('OFD.xml', f'<OFD {ns}><DocBody><DocRoot>Doc/Document.xml</DocRoot></DocBody></OFD>')
    z.writestr('Doc/Document.xml', f'<Document {ns}><CommonData><PageArea><PhysicalBox>0 0 210 297</PhysicalBox></PageArea></CommonData><Pages><Page ID="1" BaseLoc="Page.xml"/></Pages></Document>')
    z.writestr('Doc/Page.xml', f'<Page {ns}><Content><Layer ID="1"><PathObject ID="2" Boundary="4 10 202 42" Fill="true" Stroke="false"><FillColor Value="0 0 204"/><AbbreviatedData>M 0 0 L 202 0 L 202 42 L 0 42 C</AbbreviatedData></PathObject></Layer></Content></Page>')
sys.stdout.buffer.write(buf.getvalue())
`]);
  const xps = execFileSync('python3', ['-c', `
import io, sys, zipfile
buf = io.BytesIO()
ns = 'xmlns="http://schemas.microsoft.com/xps/2005/06"'
with zipfile.ZipFile(buf, 'w') as z:
    z.writestr('FixedDocSeq.fdseq', f'<FixedDocumentSequence {ns}><DocumentReference Source="Document.fdoc"/></FixedDocumentSequence>')
    z.writestr('Document.fdoc', f'<FixedDocument {ns}><PageContent Source="Page.fpage"/></FixedDocument>')
    z.writestr('Page.fpage', f'<FixedPage {ns} Width="800" Height="1100"><Path Data="M 16,40 L 784,40 L 784,200 L 16,200 Z" Fill="#FFCC0000"/></FixedPage>')
sys.stdout.buffer.write(buf.getvalue())
`]);
  const fixed = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 1, hasTouch: true, isMobile: true });
  fixed.on('pageerror', error => failures.push(error.message));
  await fixed.addInitScript(version => {
    localStorage.setItem('ibisOffice.app.last-started-version', version);
    localStorage.setItem('ibisOffice.font-setup.checked-version', version);
  }, wasmVersion);
  await fixed.route('**/synthetic.pdf', route => route.fulfill({ body: pdf, contentType: 'application/pdf' }));
  const fixedUrl = new URL(url);
  fixedUrl.searchParams.set('open', './synthetic.pdf');
  await fixed.goto(fixedUrl.href);
  await fixed.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  const assertPageFitted = async (format, channel) => {
    const deadline = Date.now() + 15_000;
    do {
      const screenshot = await fixed.screenshot();
      const fits = await fixed.evaluate(async ({ data, channel }) => {
        const img = new Image();
        img.src = `data:image/png;base64,${data}`;
        await img.decode();
        const canvas = document.createElement('canvas');
        canvas.width = img.width;
        canvas.height = img.height;
        const ctx = canvas.getContext('2d');
        ctx.drawImage(img, 0, 0);
        const matches = (x, y) => {
          const rgb = ctx.getImageData(x, y, 1, 1).data.subarray(0, 3);
          return rgb.every((value, i) => i === channel ? value > 130 : value < 80);
        };
        for (let y = 80; y < 220; y += 4) {
          if (matches(48, y) && matches(342, y) && !matches(24, y) && !matches(366, y)) return true;
        }
        return false;
      }, { data: screenshot.toString('base64'), channel });
      if (fits) return;
      await fixed.waitForTimeout(250);
    } while (Date.now() < deadline);
    if (process.env.IBIS_TOUCH_ARTIFACTS) await fixed.screenshot({ path: `${process.env.IBIS_TOUCH_ARTIFACTS}/fixed-layout-${format}.png` });
    assert.fail(`${format} must render its page fitted to the phone width`);
  };
  await assertPageFitted('PDF', 1);
  for (const [format, name, mimeType, buffer, channel] of [
    ['OFD', 'synthetic.ofd', 'application/ofd', ofd, 2],
    ['XPS', 'synthetic.xps', 'application/vnd.ms-xpsdocument', xps, 0],
  ]) {
    const chooser = fixed.waitForEvent('filechooser', { timeout: 15_000 });
    await fixed.touchscreen.tap(342, 24); // Read-only bar has only Open.
    await (await chooser).setFiles({ name, mimeType, buffer });
    await fixed.waitForFunction(name => document.title.includes(name), name);
    await assertPageFitted(format, channel);
  }
  assert.deepEqual(failures, [], 'fixed-layout previews must not panic');
  await fixed.context().close();

  // Open from the file picker, whose source is retained for the download action.
  const editing = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 1, hasTouch: true, isMobile: true });
  editing.on('pageerror', error => failures.push(error.message));
  editing.on('console', message => {
    if (message.type() === 'error' && /panicked|RuntimeError|RefCell already borrowed/.test(message.text())) failures.push(message.text());
  });
  await editing.addInitScript(version => {
    localStorage.setItem('ibisOffice.app.last-started-version', version);
    localStorage.setItem('ibisOffice.font-setup.checked-version', version);
    window.addEventListener('ibisoffice:download', event => { window.__ibisDownloaded = Array.from(event.detail.bytes); });
  }, wasmVersion);
  const editingUrl = new URL(url);
  editingUrl.searchParams.set('download', '1');
  editingUrl.searchParams.set('view', '0');
  await editing.goto(editingUrl.href);
  await editing.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
  await waitForEditorFrame(editing);
  await editing.waitForTimeout(1000);
  const editingCdp = await editing.context().newCDPSession(editing);
  const editingTouch = (type, touchPoints) => editingCdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  const editingTap = (id, x, y) => tap(editingTouch, id, x, y);
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
  await editing.waitForTimeout(300);
  // Downloading a copy leaves the edited file dirty. Acknowledge the save prompt
  // before opening another local document in this browser tab.
  await editingTap(61, 272, 477); // Don't Save
  await (await chooser).setFiles('www/samples/sorane-welcome.docx');
  await editing.waitForTimeout(3000);
  await editingTouch('touchStart', [pt(12, 300, 550)]);
  await editingTouch('touchMove', [pt(12, 300, 400)]);
  await editingTouch('touchEnd', []);
  await editing.waitForTimeout(500);
  const mirror = editing.locator('textarea');
  assert.equal(await mirror.getAttribute('inputmode'), 'none', 'a pan must not summon the virtual keyboard');
  // Multi-contact sequences must not summon the keyboard, regardless of
  // which finger is lifted first. The following single tap still may do so.
  for (const [first, second, releaseFirst] of [[40, 41, true], [50, 51, false]]) {
    await editingTouch('touchStart', [pt(first, 110, 200)]);
    await editingTouch('touchStart', [pt(first, 110, 200), pt(second, 170, 200)]);
    await editingTouch('touchMove', [pt(first, 100, 200), pt(second, 180, 200)]);
    await editingTouch('touchEnd', [releaseFirst ? pt(second, 180, 200) : pt(first, 100, 200)]);
    await editingTouch('touchEnd', []);
    await editing.waitForTimeout(300);
    assert.equal(await mirror.getAttribute('inputmode'), 'none', 'a pinch must not summon the virtual keyboard');
  }
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
