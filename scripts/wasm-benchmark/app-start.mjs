// Compare the unchanged app's normal loader with a precompressed single file.
// Usage is documented in README.md. This fixture does not modify production.
import { chromium } from 'playwright-core';
import { readFile } from 'node:fs/promises';

const { BENCH_APP_URL: appUrl, BENCH_CDN_ORIGIN: candidateOrigin, BENCH_MANIFEST: manifestPath,
  BENCH_CHROME: executablePath = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  BENCH_REPEATS: repeats = '3', BENCH_VERSION: version = '0.8.7' } = process.env;
if (!appUrl || !candidateOrigin || !manifestPath) throw new Error('Set BENCH_APP_URL, BENCH_CDN_ORIGIN and BENCH_MANIFEST');
if (!Number.isSafeInteger(Number(repeats)) || Number(repeats) < 1 || Number(repeats) > 100) throw new Error('Invalid BENCH_REPEATS');
const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
const expected = manifest.sha256;
if (!/^[a-f0-9]{64}$/.test(expected)) throw new Error('Invalid expected digest');
const current = await fetch(new URL('/pkg/ibis_office_app_bg.manifest.json', appUrl));
if (!current.ok || (await current.json()).sha256 !== expected) throw new Error('App and benchmark Wasm differ');
const splitLoaderResponse = await fetch(new URL('/wasm-loader.js', appUrl));
if (!splitLoaderResponse.ok) throw new Error('Production loader unavailable');
const splitLoader = await splitLoaderResponse.text();
const candidate = new URL('/single-br.wasm', candidateOrigin).href;
const loader = stream => `export async function loadWasm(_url,progress){
 const r=await fetch(${JSON.stringify(candidate)});if(!r.ok)throw Error('Wasm HTTP '+r.status);
 ${stream ? 'const [m,b]=await Promise.all([WebAssembly.compileStreaming(r.clone()),r.arrayBuffer()]);' : 'const b=await r.arrayBuffer();'}
 const h=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',b))).map(x=>x.toString(16).padStart(2,'0')).join('');
 if(h!==${JSON.stringify(expected)})throw Error('Wasm digest mismatch');progress(b.byteLength,b.byteLength);
 return ${stream ? 'm' : "new Response(b,{headers:{'Content-Type':'application/wasm'}})"};
}`;
const browser = await chromium.launch({ executablePath, headless: true,
  args: ['--enable-unsafe-webgpu', '--use-angle=swiftshader', '--disable-gpu-sandbox'] });
try {
  for (let round = 0; round < Number(repeats); round++) {
    const modes = round % 2 ? ['single-stream-br', 'single-br', 'split'] : ['split', 'single-br', 'single-stream-br'];
    for (const mode of modes) {
      const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
      const page = await context.newPage(), errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.addInitScript(version => { localStorage.setItem('ibisOffice.app.last-started-version', version); localStorage.setItem('ibisOffice.font-setup.checked-version', version); }, version);
      await page.route('**/wasm-loader.js', route => route.fulfill({ status: 200, contentType: 'text/javascript',
        body: mode === 'split' ? splitLoader : loader(mode === 'single-stream-br') }));
      const cdp = await context.newCDPSession(page);
      await cdp.send('Network.enable');
      await cdp.send('Network.setCacheDisabled', { cacheDisabled: true });
      await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 40, downloadThroughput: 5_000_000, uploadThroughput: 1_000_000 });
      const start = Date.now();
      await page.goto(appUrl, { waitUntil: 'domcontentloaded' });
      await page.waitForFunction(() => !document.querySelector('#status'), null, { timeout: 180_000 });
      const readyMs = Date.now() - start;
      if (errors.length) throw new Error(errors.join('; '));
      console.log(JSON.stringify({ round, mode, readyMs, errors }));
      await context.close();
    }
  }
} finally { await browser.close(); }
