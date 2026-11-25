// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
// Pure helpers for the service worker (and its node tests). No DOM, no Cache API.

export const CACHE_PREFIX = 'sorane-docs-';

export type PrecacheManifest = {
  version: string;
  revision: string;
  urls: string[];
  bytes: number;
};

export function cacheName(revision: string): string {
  return `${CACHE_PREFIX}${revision}`;
}

export function isOurCache(name: string): boolean {
  return name.startsWith(CACHE_PREFIX);
}

/** Requests the worker must not cache or intercept. */
export function shouldBypass(request: { method: string }, url: URL, origin: string): boolean {
  if (request.method !== 'GET' && request.method !== 'HEAD') return true;
  if (url.origin !== origin) return true;
  const path = url.pathname;
  if (
    path.startsWith('/auth/')
    || path.startsWith('/api/')
    || path.startsWith('/files')
    || path.startsWith('/ai/')
  ) {
    return true;
  }
  // The browser fetches sw.js itself; never answer it from a previous cache.
  if (path.endsWith('/sw.js') || path === '/sw.js') return true;
  return false;
}

export function isNavigationRequest(request: { mode?: string; destination?: string; headers?: { get(name: string): string | null } }): boolean {
  if (request.mode === 'navigate' || request.destination === 'document') return true;
  const accept = request.headers?.get('accept') ?? '';
  return accept.includes('text/html');
}

/** Content-hashed assets that belong to one build and can be served cache-first. */
export function isImmutableAsset(pathname: string): boolean {
  return (
    /\/pkg\/ibis_office_app_bg\.[a-f0-9]{64}\.chunk$/.test(pathname)
    || /\/pkg\/wasm-br\.[a-f0-9]{64}\.bin$/.test(pathname)
    || /\/pkg\/br\/[a-f0-9]{64}\.wasm$/.test(pathname)
    || /\.[a-f0-9]{12,}\.(ttf|otf|woff2?|js|css)$/.test(pathname)
  );
}

export function offlineFallbackUrl(): string {
  return './offline.html';
}

export function shouldSkipWaiting(data: unknown): boolean {
  return data === 'SKIP_WAITING' || (typeof data === 'object' && data !== null && (data as { type?: string }).type === 'SKIP_WAITING');
}
