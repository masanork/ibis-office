// SPDX-License-Identifier: AGPL-3.0-only
// Route sample-admin, authentication and private file endpoints; leave bundled
// samples and every existing editor/font asset with the original Worker and static assets.
const routes = new Set(['/admin', '/admin/enroll', '/admin/samples', '/login', '/callback', '/logout', '/backchannel', '/api/samples', '/files', '/api/files']);
export function isSoraneAdminRoute(path: string): boolean {
  return routes.has(path) || /^\/api\/samples\/[0-9a-f-]{36}\/[a-zA-Z0-9_-]{1,80}\.(?:docx|xlsx|pptx)$/.test(path) || /^\/api\/files\/[0-9a-f-]{36}\/[A-Za-z0-9._-]{1,80}\.(?:docx|xlsx|pptx)$/i.test(path);
}
export async function dispatchSoraneAdmin(request: Request, service?: { fetch(request: Request): Promise<Response> }): Promise<Response | null> {
  const url = new URL(request.url);
  if (!isSoraneAdminRoute(url.pathname)) return null;
  if (!service) return new Response('Not found', {status: 404, headers: {'Cache-Control': 'no-store'}});
  if (url.hostname === 'ibis.sorane.dev') {
    url.hostname = 'docs.sorane.dev';
    // Authentication cookies are scoped to the canonical RP origin. Public
    // sample reads remain same-origin for the legacy ibis.sorane.dev editor.
    if (request.method === 'GET' && (url.pathname === '/api/samples' || url.pathname.startsWith('/api/samples/'))) {
      return service.fetch(new Request(url.toString(), request));
    }
    if (request.method === 'GET') return Response.redirect(url.toString(), 308);
    return new Response('Not found', {status: 404, headers: {'Cache-Control': 'no-store'}});
  }
  return service.fetch(request);
}
