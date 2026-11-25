// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  cacheName,
  isImmutableAsset,
  isOurCache,
  isNavigationRequest,
  shouldBypass,
  shouldSkipWaiting,
  offlineFallbackUrl,
} from '../../www/pwa-sw-logic.js';

const origin = 'https://docs.sorane.dev';

test('cache names are versioned and old caches can be recognized', () => {
  assert.equal(cacheName('abc123'), 'sorane-docs-abc123');
  assert.ok(isOurCache('sorane-docs-abc123'));
  assert.equal(isOurCache('workbox-precache'), false);
});

test('API, auth, file, AI, and cross-origin requests bypass the cache', () => {
  const get = { method: 'GET' };
  assert.equal(shouldBypass(get, new URL('/manifest.webmanifest', origin), origin), false);
  assert.equal(shouldBypass(get, new URL('/sw.js', origin), origin), true);
  assert.equal(shouldBypass({ method: 'POST' }, new URL('/pkg/x', origin), origin), true);
  assert.equal(shouldBypass(get, new URL('https://example.com/pkg/x'), origin), true);
  for (const path of ['/auth/github/login', '/api/github/file', '/files/doc.docx', '/ai/complete']) {
    assert.equal(shouldBypass(get, new URL(path, origin), origin), true, path);
  }
});

test('hashed wasm and font files are immutable; HTML is a navigation', () => {
  assert.ok(isImmutableAsset('/pkg/ibis_office_app_bg.' + 'a'.repeat(64) + '.chunk'));
  assert.ok(isImmutableAsset('/pkg/br/' + 'b'.repeat(64) + '.wasm'));
  assert.ok(isImmutableAsset('/fonts/ACGJMincho/base.6b0d2674671a.ttf'));
  assert.equal(isImmutableAsset('/main.js'), false);
  assert.equal(isImmutableAsset('/index.html'), false);
  assert.ok(isNavigationRequest({ mode: 'navigate' }));
  assert.ok(isNavigationRequest({ destination: 'document' }));
  assert.ok(isNavigationRequest({ headers: { get: (n) => n === 'accept' ? 'text/html,application/xhtml+xml' : null } }));
  assert.equal(isNavigationRequest({ destination: 'script', headers: { get: () => '*/*' } }), false);
  assert.equal(offlineFallbackUrl(), './offline.html');
});

test('skipWaiting runs only when the page asks', () => {
  assert.equal(shouldSkipWaiting('SKIP_WAITING'), true);
  assert.equal(shouldSkipWaiting({ type: 'SKIP_WAITING' }), true);
  assert.equal(shouldSkipWaiting('install'), false);
  assert.equal(shouldSkipWaiting(undefined), false);
});
