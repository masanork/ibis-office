// SPDX-License-Identifier: AGPL-3.0-only
/** A precompressed asset is served only to a client accepting Brotli. */
function acceptsBrotli(header: string | null): boolean {
  if (!header) return false;
  let wildcard: number | undefined;
  for (const part of header.split(',')) {
    const [name, ...parameters] = part.trim().toLowerCase().split(';');
    let quality = 1;
    for (const param of parameters) {
      const match = /^\s*q\s*=\s*(0(?:\.\d{0,3})?|1(?:\.0{0,3})?)\s*$/.exec(param);
      if (match) quality = Number(match[1]);
      else if (/^\s*q\s*=/.test(param)) quality = 0;
    }
    if (name.trim() === 'br') return quality > 0;
    if (name.trim() === '*') wildcard = quality;
  }
  return (wildcard ?? 0) > 0;
}

type AssetFetcher = { fetch(request: Request): Promise<Response> };

/** Return null for unrelated routes; a rejected encoding falls back in the client. */
export async function serveBrotliWasm(request: Request, assets: AssetFetcher): Promise<Response | null> {
  const url = new URL(request.url);
  const match = /^\/pkg\/br\/([a-f0-9]{64})\.wasm$/.exec(url.pathname);
  if (!match) return null;
  if (request.method !== 'GET' && request.method !== 'HEAD') return new Response('Method not allowed', { status: 405, headers: { Allow: 'GET, HEAD' } });
  // Cloudflare may rewrite the forwarded header to `br, gzip`; cf carries
  // what the browser actually sent. Tests and non-CF runtimes use the header.
  const original = (request as Request & { cf?: { clientAcceptEncoding?: string } }).cf?.clientAcceptEncoding;
  if (!acceptsBrotli(original ?? request.headers.get('Accept-Encoding'))) return new Response('Brotli not accepted', { status: 406, headers: { Vary: 'Accept-Encoding', 'Cache-Control': 'no-store' } });
  const assetUrl = new URL(`/pkg/wasm-br.${match[1]}.bin`, url);
  // ASSETS receives the stored compressed file as opaque bytes. Avoid asking
  // the asset binding to apply its own compression before streaming it.
  const asset = await assets.fetch(new Request(assetUrl, { method: request.method, headers: { 'Accept-Encoding': 'identity' } }));
  if (!asset.ok) return new Response('Wasm asset unavailable', { status: asset.status, headers: { 'Cache-Control': 'no-store' } });
  const encoding = asset.headers.get('Content-Encoding');
  if (encoding && encoding !== 'identity') return new Response('Unexpected asset encoding', { status: 502, headers: { 'Cache-Control': 'no-store' } });
  const headers = new Headers({ 'Content-Type': 'application/wasm', 'Content-Encoding': 'br',
    'Cache-Control': 'public, max-age=31536000, immutable', Vary: 'Accept-Encoding' });
  return new Response(request.method === 'HEAD' ? null : asset.body, { status: 200, headers, encodeBody: 'manual' } as ResponseInit & { encodeBody: 'manual' });
}
