// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dispatchSoraneAdmin, isSoraneAdminRoute } from '../../worker/sorane-gateway.js';
test('only admin and published dynamic sample URLs reach the service', async () => {
  assert.equal(isSoraneAdminRoute('/api/samples'), true);
  assert.equal(isSoraneAdminRoute('/files'), true);
  assert.equal(isSoraneAdminRoute('/api/files'), true);
  assert.equal(isSoraneAdminRoute('/api/files/123e4567-e89b-12d3-a456-426614174000/welcome.docx'), true);
  assert.equal(isSoraneAdminRoute('/api/files/not-an-id/welcome.docx'), false);
  assert.equal(isSoraneAdminRoute('/samples/sorane-welcome.docx'), false);
  assert.equal(isSoraneAdminRoute('/api/font/subset'), false);
  assert.equal(isSoraneAdminRoute('/api/samples/not-an-id/test.docx'), false);
  const request = new Request('https://docs.example/admin');
  assert.equal((await dispatchSoraneAdmin(request)).status, 404);
  let seen; const response = await dispatchSoraneAdmin(request, {fetch: async r => {seen=r;return new Response('admin')}});
  assert.equal(seen, request); assert.equal(await response.text(), 'admin');
  assert.equal(await dispatchSoraneAdmin(new Request('https://docs.example/editor.html')), null);
});
test('legacy domain reads public samples while login uses the canonical origin', async () => {
  let forwarded;
  const service = { fetch: async r => {forwarded = r; return new Response('ok');} };
  const sample = new Request('https://ibis.sorane.dev/api/samples');
  assert.equal((await dispatchSoraneAdmin(sample, service)).status, 200);
  assert.equal(forwarded.url, 'https://docs.sorane.dev/api/samples');
  const admin = await dispatchSoraneAdmin(new Request('https://ibis.sorane.dev/admin'), service);
  assert.equal(admin.status, 308); assert.equal(admin.headers.get('location'), 'https://docs.sorane.dev/admin');
  const files = await dispatchSoraneAdmin(new Request('https://ibis.sorane.dev/files'), service);
  assert.equal(files.status, 308); assert.equal(files.headers.get('location'), 'https://docs.sorane.dev/files');
  const privateFile = await dispatchSoraneAdmin(new Request('https://ibis.sorane.dev/api/files/123e4567-e89b-12d3-a456-426614174000/welcome.docx'), service);
  assert.equal(privateFile.status, 308); assert.equal(privateFile.headers.get('location'), 'https://docs.sorane.dev/api/files/123e4567-e89b-12d3-a456-426614174000/welcome.docx');
  assert.equal((await dispatchSoraneAdmin(new Request('https://ibis.sorane.dev/login',{method:'POST'}), service)).status, 404);
  assert.equal((await dispatchSoraneAdmin(new Request('https://ibis.sorane.dev/api/files',{method:'POST'}), service)).status, 404);
});
