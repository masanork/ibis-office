// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isServiceWorkerRequest, serveServiceWorker } from '../../worker/sw-headers.js';

test('only the service worker script is rewritten', () => {
  assert.equal(isServiceWorkerRequest('/sw.js'), true);
  assert.equal(isServiceWorkerRequest('/editor/sw.js'), true);
  assert.equal(isServiceWorkerRequest('/main.js'), false);
});

test('the worker serves sw.js with no-cache and a root scope', async () => {
  const assets = {
    async fetch() {
      return new Response('// sw', { headers: { 'Content-Type': 'text/javascript', 'Cache-Control': 'public, max-age=3600' } });
    },
  };
  const response = await serveServiceWorker(new Request('https://docs.sorane.dev/sw.js'), assets);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('Cache-Control'), 'no-cache, no-store, must-revalidate');
  assert.equal(response.headers.get('Service-Worker-Allowed'), '/');
  assert.match(response.headers.get('Content-Type'), /javascript/);
  assert.equal(await response.text(), '// sw');
  assert.equal(await serveServiceWorker(new Request('https://docs.sorane.dev/main.js'), assets), null);
  const denied = await serveServiceWorker(new Request('https://docs.sorane.dev/sw.js', { method: 'POST' }), assets);
  assert.equal(denied.status, 405);
});
