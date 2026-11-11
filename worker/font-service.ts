// SPDX-License-Identifier: AGPL-3.0-only
export interface FontSpec {
  id: string;
  key: string;
  license: string;
  originalUrl: string;
}
export interface SourceInfo { size: number; etag: string }
export interface SourceStore {
  head(key: string): Promise<SourceInfo | null>;
  read(key: string, etag: string): Promise<ReadableStream<Uint8Array> | null>;
}
export interface SubsetEngine {
  append(bytes: Uint8Array): void;
  has_glyphs(text: string): boolean;
  subset(text: string, woff2: boolean): Uint8Array;
  free(): void;
}
export interface SubsetCache {
  match(request: Request): Promise<Response | undefined>;
  put(request: Request, response: Response): Promise<void>;
}
export interface FontRequestContext {
  store: SourceStore;
  cache: SubsetCache;
  allowed(): Promise<boolean>;
  waitUntil(promise: Promise<unknown>): void;
}

const cors = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Expose-Headers': 'X-Font-Id, X-Font-License, X-Original-Font, X-Font-Missing, X-Font-Boundaries, ETag, X-Cache',
};
const error = (message: string, status: number, extra = {}) => Response.json(
  { error: message }, { status, headers: { ...cors, 'Cache-Control': 'no-store', ...extra } });
const hash = async (data: string | Uint8Array) => [...new Uint8Array(await crypto.subtle.digest(
  'SHA-256', typeof data === 'string' ? new TextEncoder().encode(data) : new Uint8Array(data)
))].map(n => n.toString(16).padStart(2, '0')).join('');

/** Owns only the shared WASM resource, never an environment, request or I/O
 * promise. One active source prevents simultaneous large-font allocations. */
export function createFontService(fonts: readonly FontSpec[], version: string,
                                  create: (size: number) => SubsetEngine,
                                  now: () => number = Date.now) {
  type Lease = { expires: number; engine?: SubsetEngine };
  let active: Lease | undefined;
  const release = (lease: Lease) => {
    const engine = lease.engine;
    lease.engine = undefined;
    if (active === lease) active = undefined;
    engine?.free();
  };
  return async (request: Request, context: FontRequestContext): Promise<Response> => {
    if (request.method === 'OPTIONS') return new Response(null, { headers: {
      ...cors, 'Access-Control-Allow-Methods': 'GET, OPTIONS', 'Access-Control-Max-Age': '86400',
    } });
    if (request.method !== 'GET') return error('Use GET', 405, { Allow: 'GET, OPTIONS' });
    const url = new URL(request.url);
    const font = fonts.find(f => f.id === (url.searchParams.get('font') ?? 'ipamjm'));
    if (!font) return error('Unknown font', 404);
    const text = url.searchParams.get('text');
    if (!text || text.length > 2048 || [...text].length > 1024) return error('text must contain 1 to 1024 code points', 400);
    const format = url.searchParams.get('format') ?? 'otf';
    if (!['otf', 'woff2'].includes(format)) return error('Unsupported font format', 400);
    const partial = url.searchParams.get('partial') === '1';
    if (!await context.allowed()) return error('Too many font requests', 429, { 'Retry-After': '1' });

    let lease: Lease | undefined;
    try {
      const info = await context.store.head(font.key);
      if (!info) return error('Source font is unavailable', 503);
      if (!Number.isSafeInteger(info.size) || info.size < 12 || info.size > 50 * 1024 * 1024) return error('Source font size is unsupported', 503);
      const key = new Request(url.origin + '/__font-cache/' + await hash(JSON.stringify([version, font.id, info.etag, format, text, partial])));
      const cached = await context.cache.match(key);
      if (cached) {
        const response = new Response(cached.body, cached);
        response.headers.set('X-Cache', 'HIT');
        return response;
      }
      // Request cancellation can skip its JS finally block. Keep the WASM
      // resource recoverable without retaining request-owned I/O globally.
      if (active && now() >= active.expires) release(active);
      if (active) return error('Font engine is busy', 429, { 'Retry-After': '1' });
      lease = { expires: now() + 30_000 };
      active = lease;
      const assertActive = () => { if (active !== lease) throw new Error('Font engine lease expired'); };
      const stream = await context.store.read(font.key, info.etag);
      if (!stream) return error('Source font changed; retry', 503, { 'Retry-After': '1' });
      const reader = stream.getReader();
      let count = 0;
      try {
        assertActive();
        const engine = create(info.size);
        lease.engine = engine;
        for (;;) {
          const { done, value } = await reader.read();
          assertActive();
          if (done) break;
          count += value.byteLength;
          if (count > info.size) throw new Error('Font stream exceeds source size');
          engine.append(value);
        }
      } catch (failure) {
        await reader.cancel().catch(() => {});
        throw failure;
      } finally { reader.releaseLock(); }
      if (count !== info.size) throw new Error('Incomplete source font stream');
      assertActive();
      const engine = lease.engine!;
      let subsetText = text;
      let missing: string | undefined;
      let boundaries: string | undefined;
      if (!engine.has_glyphs(text)) {
        if (!partial) return new Response(null, { status: 204, headers: {
          ...cors, 'X-Font-Id': font.id, 'Cache-Control': 'no-store',
        } });
        const segments = [...new Intl.Segmenter('en', { granularity: 'grapheme' }).segment(text)].map(item => item.segment);
        let offset = 0;
        boundaries = segments.map(segment => offset += segment.length).join(',');
        const absent: number[] = [];
        subsetText = segments.filter((segment, index) => {
          if (engine.has_glyphs(segment)) return true;
          absent.push(index); return false;
        }).join('');
        missing = absent.join(',');
        if (!subsetText) return new Response(null, { status: 204, headers: {
          ...cors, 'X-Font-Id': font.id, 'X-Font-Missing': missing, 'X-Font-Boundaries': boundaries, 'Cache-Control': 'no-store',
        } });
      }
      const output = new Uint8Array(engine.subset(subsetText, format === 'woff2'));
      if (!output.length) throw new Error('Font subsetting produced no output');
      const response = new Response(output, { headers: {
        ...cors,
        'Content-Type': format === 'woff2' ? 'font/woff2' : 'font/otf',
        'Cache-Control': 'public, max-age=3600',
        ETag: '"' + await hash(output) + '"',
        'X-Font-Id': font.id,
        ...(missing === undefined ? {} : { 'X-Font-Missing': missing }),
        ...(boundaries === undefined ? {} : { 'X-Font-Boundaries': boundaries }),
        'X-Font-License': font.license,
        'X-Original-Font': font.originalUrl,
        'X-Cache': 'MISS',
      } });
      context.waitUntil(context.cache.put(key, response.clone()).catch(() => {
        console.error(JSON.stringify({ event: 'font-cache-failed', font: font.id }));
      }));
      return response;
    } catch (failure) {
      // Do not log requested characters or URLs.
      console.error(JSON.stringify({ event: 'font-subset-failed', font: font.id,
        message: failure instanceof Error ? failure.message : 'Subset engine error' }));
      return error('Font subset failed', 503, { 'Retry-After': '1' });
    } finally {
      if (lease) release(lease);
    }
  };
}
