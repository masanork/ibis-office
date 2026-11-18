// SPDX-License-Identifier: AGPL-3.0-only
// Run against a Web build serving scripts/web-fonts.sorane.json.
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { loadSubsetter } from './font-subsetter.mjs';
import { chromium } from 'playwright-core';
import { wasmVersion } from './browser-test-version.mjs';

const origin = process.env.IBIS_MERMAID_TEST_ORIGIN ?? 'http://127.0.0.1:8787';
const executablePath = process.env.IBIS_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({
  executablePath, headless: true,
  args: ['--disable-webgpu', '--use-angle=swiftshader', '--disable-gpu-sandbox'],
});
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 900 }, deviceScaleFactor: 1, colorScheme: 'light' });
  const missing = [];
  const fontErrors = [];
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  page.on('console', message => {
    const text = message.text();
    if (text.includes('Could not load a dynamic font')) fontErrors.push(text);
    if (/usvg::text.*(?:No match for|No fonts with)/.test(text)) missing.push(text);
  });
  await page.addInitScript(version => {
    localStorage.setItem('ibisoffice-lang', 'ja');
    localStorage.setItem('ibisOffice.app.last-started-version', version);
    localStorage.setItem('ibisOffice.font-setup.checked-version', version);
  }, wasmVersion);
  const failure = process.env.IBIS_MERMAID_TEST_FONT_FAILURE;
  const dynamic = failure || process.env.IBIS_MERMAID_TEST_DYNAMIC === '1';
  assert.ok(!failure || ['unsupported', 'unavailable', 'retry'].includes(failure));
  assert.ok(!failure || process.env.IBIS_MERMAID_TEST_FONT_FIXTURE === '1');
  const file = dynamic ? '/samples/mermaid-dynamic-labels.md' : '/samples/mermaid-labels.md';
  const fontService = process.env.IBIS_MERMAID_TEST_FONT_ORIGIN;
  let injectedErrors = 0;
  if (process.env.IBIS_MERMAID_TEST_FONT_FIXTURE === '1') {
    // Exercise font registration with real subsets without depending on production.
    const subsetter = await loadSubsetter();
    const sourceDir = new URL('../data/fonts/', import.meta.url);
    const catalog = JSON.parse(await readFile(new URL('../fonts/sources.json', import.meta.url), 'utf8'));
    const fonts = new Map(await Promise.all(catalog.fonts.map(async ({ id, filename }) => [
      id, await readFile(new URL(filename, sourceDir)),
    ])));
    await page.route(url => url.pathname === '/api/font/subset', async route => {
      const { searchParams } = new URL(route.request().url());
      const face = searchParams.get('font');
      const font = fonts.get(face);
      const glyphs = searchParams.get('text') ?? '';
      if (glyphs.includes('𠮷')) {
        if (failure === 'unsupported') {
          await route.fulfill({ status: 204 });
          return;
        }
        if (failure === 'unavailable' || (failure === 'retry' && injectedErrors === 0)) {
          injectedErrors++;
          await route.fulfill({ status: 503 });
          return;
        }
      }
      if (!font || !glyphs || !subsetter.font_has_glyphs(font, glyphs)) {
        await route.fulfill({ status: 204 });
        return;
      }
      await route.fulfill({
        status: 200,
        headers: { 'X-Font-Id': face, 'Content-Type': 'font/otf' },
        body: Buffer.from(subsetter.subset_font_raw(font, glyphs)),
      });
    });
  } else if (fontService) {
    await page.route(url => url.pathname === '/api/font/subset', async route => {
      const request = new URL(route.request().url());
      const response = await route.fetch({ url: new URL(request.pathname + request.search, fontService).href });
      await route.fulfill({ response });
    });
  }
  const requests = [];
  page.on('request', request => {
    const url = new URL(request.url());
    if (url.pathname === '/api/font/subset') requests.push(url.searchParams.get('text') ?? '');
  });
  await Promise.all([
    page.waitForResponse(response => new URL(response.url()).pathname === file && response.ok()),
    page.goto(new URL('/editor.html?open=' + encodeURIComponent(file), origin).href),
  ]);
  await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });

  // The light-blue interiors identify the three Mermaid nodes. Wait for their
  // rasterized image, then require dark label ink inside each of the three nodes.
  let boxes = [];
  let screenshot;
  for (let attempt = 0; attempt < 40; attempt++) {
    // Capture the composited frame, as in the touch browser test.
    screenshot = await page.screenshot();
    boxes = await page.evaluate(async data => {
      const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${data}`)).blob());
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d');
      ctx.drawImage(bitmap, 0, 0);
      const { data: pixels } = ctx.getImageData(0, 0, canvas.width, canvas.height);
      const blue = (x, y) => {
        const i = 4 * (y * canvas.width + x);
        return pixels[i] >= 233 && pixels[i] <= 242 && pixels[i + 1] >= 240 && pixels[i + 1] <= 248 && pixels[i + 2] >= 250;
      };
      for (let y = 0; y < canvas.height - 22; y++) {
        const spans = [];
        for (let x = 0; x < canvas.width;) {
          const start = x;
          while (x < Math.min(900, canvas.width) && blue(x, y)) x++;
          if (x - start >= 40) spans.push([start, x]);
          if (x === start) x++;
        }
        if (spans.length !== 3) continue;
        return spans.map(([left, right]) => {
          let ink = 0;
          for (let row = y + 5; row < y + 22; row++) {
            for (let column = left + 10; column < right - 10; column++) {
              const i = 4 * (row * canvas.width + column);
              if (pixels[i] < 100 && pixels[i + 1] < 110 && pixels[i + 2] < 130) ink++;
            }
          }
          return ink;
        });
      }
      return [];
    }, screenshot.toString('base64'));
    if (boxes.length === 3 && boxes[0] > 0 && boxes[2] > 0 &&
        (failure === 'unavailable' ? fontErrors.length > 0 : failure === 'unsupported' || boxes[1] > 0)) break;
    await page.waitForTimeout(300);
  }
  try {
    assert.deepEqual(pageErrors, [], 'the editor should open without a browser exception');
    assert.equal(boxes.length, 3, 'the Mermaid nodes should be visible');
    if (dynamic) assert.ok(requests.some(body => body.includes('𠮷')), 'the SVG-only glyph must be requested');
    if (failure === 'unsupported' || failure === 'unavailable') {
      assert.ok(boxes[0] > 0 && boxes[2] > 0, `label pixels per node: ${boxes}`);
      if (failure === 'unavailable') {
        assert.ok(injectedErrors >= 3, 'persistent failure must exhaust the three attempts');
        assert.ok(fontErrors.length > 0, 'persistent failure must be reported');
      } else {
        assert.deepEqual(fontErrors, []);
      }
      console.log(`PASS: diagram and neighboring labels render with font service ${failure} (label pixels: ${boxes})`);
    } else {
      assert.ok(boxes.every(ink => ink > 0), `label pixels per node: ${boxes}`);
      if (failure === 'retry') assert.equal(injectedErrors, 1);
      assert.deepEqual(missing, []);
      assert.deepEqual(fontErrors, []);
      console.log('PASS: English, Japanese and symbol SVG labels render without missing-glyph warnings');
    }
  } catch (error) {
    if (process.env.IBIS_MERMAID_TEST_SCREENSHOT) {
      await writeFile(process.env.IBIS_MERMAID_TEST_SCREENSHOT, screenshot);
    }
    throw error;
  }
} finally {
  await browser.close();
}
