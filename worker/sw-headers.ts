// SPDX-License-Identifier: AGPL-3.0-only
/** Serve the service worker with no-cache and a root scope. */

export function isServiceWorkerRequest(pathname: string): boolean {
  return pathname === '/sw.js' || pathname.endsWith('/sw.js');
}

type AssetFetcher = { fetch(request: Request): Promise<Response> };

export async function serveServiceWorker(request: Request, assets: AssetFetcher): Promise<Response | null> {
  const url = new URL(request.url);
  if (!isServiceWorkerRequest(url.pathname)) return null;
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return new Response('Method not allowed', { status: 405, headers: { Allow: 'GET, HEAD', 'Cache-Control': 'no-store' } });
  }
  const asset = await assets.fetch(request);
  const headers = new Headers(asset.headers);
  headers.set('Cache-Control', 'no-cache, no-store, must-revalidate');
  headers.set('Service-Worker-Allowed', '/');
  headers.set('Content-Type', 'application/javascript; charset=utf-8');
  return new Response(request.method === 'HEAD' ? null : asset.body, { status: asset.status, headers });
}
