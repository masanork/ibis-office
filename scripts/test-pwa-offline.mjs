// SPDX-License-Identifier: AGPL-3.0-only
// Headless check: the service worker registers, the shell loads offline, and the update banner reloads on request.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, writeFile, stat } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { extname, join, normalize } from 'node:path';
import { chromium } from 'playwright-core';

const root = new URL('../www/', import.meta.url);
const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.json': 'application/json',
  '.webmanifest': 'application/manifest+json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.wasm': 'application/wasm',
  '.chunk': 'application/wasm',
  '.bin': 'application/octet-stream',
  '.ttf': 'font/ttf',
  '.md': 'text/markdown; charset=utf-8',
};

function chromePath() {
  return process.env.IBIS_CHROME_PATH
    || (process.platform === 'darwin'
      ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
      : '/usr/bin/google-chrome');
}

async function listen(dir) {
  const server = createServer(async (req, res) => {
    const path = new URL(req.url ?? '/', 'http://127.0.0.1').pathname;
    let rel = path === '/' ? 'index.html' : path.slice(1);
    rel = normalize(rel).replace(/^(\.\.[/\\])+/, '');
    const file = join(dir, rel);
    try {
      const info = await stat(file);
      if (!info.isFile()) throw new Error('not a file');
      res.writeHead(200, { 'Content-Type': types[extname(file)] ?? 'application/octet-stream', 'Cache-Control': path.endsWith('/sw.js') ? 'no-cache' : 'max-age=0, must-revalidate' });
      createReadStream(file).pipe(res);
    } catch {
      res.writeHead(404);
      res.end();
    }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  return { server, origin: `http://127.0.0.1:${port}` };
}

const swPath = new URL('../www/sw.js', import.meta.url);
await stat(swPath).catch(() => {
  throw new Error('www/sw.js is missing; run scripts/build-pwa-sw.mjs after a web build');
});

const started = process.env.IBIS_PWA_TEST_URL ? null : await listen(new URL('../www', import.meta.url).pathname);
const origin = process.env.IBIS_PWA_TEST_URL ?? started.origin;
const browser = await chromium.launch({
  executablePath: chromePath(),
  headless: true,
  args: ['--disable-gpu-sandbox'],
});
const originalSw = await readFile(swPath, 'utf8');
try {
  const context = await browser.newContext();
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(origin + '/', { waitUntil: 'domcontentloaded', timeout: 60_000 });
  const pageErrors = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error') pageErrors.push(msg.text());
  });
  await page.waitForFunction(async () => {
    const reg = await navigator.serviceWorker.getRegistration();
    return !!(reg && (reg.active || reg.installing || reg.waiting));
  }, null, { timeout: 180_000 });
  await Promise.race([
    page.evaluate(() => navigator.serviceWorker.ready.then(() => undefined)),
    page.waitForTimeout(60_000).then(() => { throw new Error('serviceWorker.ready timed out; console=' + pageErrors.join(' | ')); }),
  ]);
  const cacheCount = await page.evaluate(async () => (await caches.keys()).filter((k) => k.startsWith('sorane-docs-')).length);
  assert.ok(cacheCount >= 1, 'a versioned app-shell cache must exist');

  await context.setOffline(true);
  await page.reload({ waitUntil: 'domcontentloaded', timeout: 60_000 });
  const title = await page.title();
  assert.match(title, /soraneDocs/, `offline reload should keep the app shell, got ${title}`);
  const offlineText = await page.locator('#status-text, h1').first().textContent();
  assert.ok(offlineText && offlineText.length > 0, 'the shell or the offline page must render');
  await context.setOffline(false);

  const bumped = originalSw.replace(/"revision":"[0-9a-f]{16}"/, (m) => m.replace(/[0-9a-f]{16}/, 'ffffffffffffffff'));
  assert.ok(bumped !== originalSw, 'the generated service worker must contain a revision');
  await writeFile(swPath, bumped);
  await page.evaluate(async () => {
    const reg = await navigator.serviceWorker.getRegistration();
    await reg?.update();
  });
  await page.waitForSelector('#pwa-update-banner button', { timeout: 60_000 });
  const reloaded = page.waitForNavigation({ waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.click('#pwa-update-banner button');
  await reloaded;
  const after = await page.title();
  assert.match(after, /soraneDocs/);
  assert.deepEqual(errors.filter((e) => !/service worker|Failed to load|net::ERR/i.test(e)), []);
  console.log('PASS: service worker registers, serves the shell offline, and reloads when a new version is accepted');
} finally {
  await writeFile(swPath, originalSw);
  await browser.close();
  if (started) started.server.close();
}
