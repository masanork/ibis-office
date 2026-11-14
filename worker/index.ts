// SPDX-License-Identifier: AGPL-3.0-only
import module from '../crates/ibis-font-subsetter/pkg/ibis_font_subsetter_bg.wasm';
import * as subsetter from '../crates/ibis-font-subsetter/pkg/ibis_font_subsetter_bg.js';
import build from '../crates/ibis-font-subsetter/pkg/build.json';
import sources from '../fonts/sources.json';
import { createFontService } from './font-service';
import { dispatchSoraneAdmin } from './sorane-gateway';
import { serveBrotliWasm } from './wasm-delivery';

const instance = new WebAssembly.Instance(module, { './ibis_font_subsetter_bg.js': subsetter });
subsetter.__wbg_set_wasm(instance.exports);
if (typeof instance.exports.__wbindgen_start === 'function') instance.exports.__wbindgen_start();
const serveFont = createFontService(sources.fonts, build.sha256, size => new subsetter.FontSubsetter(size));

export default {
  async fetch(request: Request, env: CloudflareEnv, ctx: ExecutionContext): Promise<Response> {
    const wasmResponse = await serveBrotliWasm(request, env.ASSETS);
    if (wasmResponse) return wasmResponse;
    const adminResponse = await dispatchSoraneAdmin(request, env.SORANE_ADMIN);
    if (adminResponse) return adminResponse;
    if (new URL(request.url).pathname !== '/api/font/subset') return env.ASSETS.fetch(request);
    const work = serveFont(request, {
      store: {
        head: key => env.FONT_SOURCES.head(key),
        async read(key, etag) {
          const source = await env.FONT_SOURCES.get(key, { onlyIf: { etagMatches: etag } });
          return source && 'body' in source ? source.body : null;
        },
      },
      cache: caches.default,
      allowed: async () => (await env.FONT_LIMITER.limit({ key: request.headers.get('CF-Connecting-IP') ?? 'local' })).success,
      waitUntil: work => ctx.waitUntil(work),
    });
    // Finish the bounded R2/WASM operation even when a tab closes while the
    // font is loading, so shared memory and its lease are always released.
    ctx.waitUntil(work.then(() => {}, () => {}));
    return work;
  },
} satisfies ExportedHandler<CloudflareEnv>;
