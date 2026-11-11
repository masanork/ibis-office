// SPDX-License-Identifier: AGPL-3.0-only
// Route only the sample service endpoints; leave bundled samples and every
// existing editor/font asset with the original Worker and static assets.
const routes = new Set(['/admin', '/admin/enroll', '/admin/samples', '/login', '/callback', '/logout', '/backchannel', '/api/samples']);
export function isSoraneAdminRoute(path: string): boolean {
  return routes.has(path) || /^\/api\/samples\/[0-9a-f-]{36}\/[a-zA-Z0-9_-]{1,80}\.(?:docx|xlsx|pptx)$/.test(path);
}
export async function dispatchSoraneAdmin(request: Request, service?: { fetch(request: Request): Promise<Response> }): Promise<Response | null> {
  if (!isSoraneAdminRoute(new URL(request.url).pathname)) return null;
  if (!service) return new Response('Not found', {status: 404, headers: {'Cache-Control': 'no-store'}});
  return service.fetch(request);
}
