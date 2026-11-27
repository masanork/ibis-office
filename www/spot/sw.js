// SPDX-License-Identifier: AGPL-3.0-only
const CACHE_NAME = 'sorane-spot-poc-v6';
const PRECACHE_URLS = [
  './',
  './index.html',
  './app.js',
  './cose-envelope.js',
  './export.js',
  './qr.js',
  './tenant-setup.js',
  './manifest.webmanifest',
  './sample-manifest.json',
];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(CACHE_NAME).then((cache) => cache.addAll(PRECACHE_URLS)),
  );
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((k) => k !== CACHE_NAME && k.startsWith('sorane-spot-poc-')).map((k) => caches.delete(k))),
    ),
  );
  self.clients.claim();
});

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  if (event.request.method !== 'GET' || url.origin !== self.location.origin) return;
  if (url.pathname.startsWith('/api/')) return;
  event.respondWith(
    caches.match(event.request).then((cached) => cached || fetch(event.request)),
  );
});
