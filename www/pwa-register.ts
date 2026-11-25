// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
// Registers the service worker and shows a reload banner when a new version waits.

const BANNER_ID = 'pwa-update-banner';
const OFFLINE_ID = 'pwa-offline-banner';

export type RegisterOptions = {
  scriptUrl?: string;
  document?: Document;
  japanese?: boolean;
  reload?: () => void;
};

export function bannerText(kind: 'update' | 'offline', japanese: boolean): string {
  if (kind === 'update') {
    return japanese ? '新しいバージョンがあります。再読み込み' : 'A new version is available. Reload';
  }
  return japanese ? 'オフラインです。開いているファイルは編集できます。' : 'You are offline. Open local files can still be edited.';
}

function ensureBanner(doc: Document, id: string, className: string): HTMLElement {
  const existing = doc.getElementById(id);
  if (existing) return existing;
  const el = doc.createElement('div');
  el.id = id;
  el.className = className;
  el.hidden = true;
  doc.body.appendChild(el);
  return el;
}

export function showUpdateBanner(
  registration: { waiting: ServiceWorker | null },
  options: RegisterOptions = {},
  onAccept?: () => void,
): void {
  const waiting = registration.waiting;
  if (!waiting) return;
  const doc = options.document ?? document;
  const japanese = options.japanese ?? (doc.documentElement.lang || '').toLowerCase().startsWith('ja');
  const banner = ensureBanner(doc, BANNER_ID, 'pwa-banner pwa-banner-update');
  banner.replaceChildren();
  const button = doc.createElement('button');
  button.type = 'button';
  button.textContent = bannerText('update', japanese);
  button.addEventListener('click', () => {
    onAccept?.();
    waiting.postMessage({ type: 'SKIP_WAITING' });
  });
  banner.appendChild(button);
  banner.hidden = false;
}

export function showOfflineBanner(offline: boolean, options: RegisterOptions = {}): void {
  const doc = options.document ?? document;
  const japanese = options.japanese ?? (doc.documentElement.lang || '').toLowerCase().startsWith('ja');
  const banner = ensureBanner(doc, OFFLINE_ID, 'pwa-banner pwa-banner-offline');
  banner.replaceChildren();
  if (!offline) {
    banner.hidden = true;
    return;
  }
  const text = doc.createElement('span');
  text.textContent = bannerText('offline', japanese);
  banner.appendChild(text);
  banner.hidden = false;
}

export async function registerServiceWorker(options: RegisterOptions = {}): Promise<ServiceWorkerRegistration | null> {
  if (!('serviceWorker' in navigator)) return null;
  const scriptUrl = options.scriptUrl ?? new URL('./sw.js', document.baseURI).href;
  const registration = await navigator.serviceWorker.register(scriptUrl, { scope: './' });
  const reload = options.reload ?? (() => window.location.reload());
  // Reload only after the user accepts a waiting worker. The first claim()
  // also fires controllerchange and must not bounce the page.
  let awaitingReload = false;
  navigator.serviceWorker.addEventListener('controllerchange', () => {
    if (!awaitingReload) return;
    awaitingReload = false;
    reload();
  });
  const accept = () => { awaitingReload = true; };
  if (registration.waiting) showUpdateBanner(registration, options, accept);
  registration.addEventListener('updatefound', () => {
    const worker = registration.installing;
    if (!worker) return;
    worker.addEventListener('statechange', () => {
      if (worker.state === 'installed' && navigator.serviceWorker.controller) {
        showUpdateBanner(registration, options, accept);
      }
    });
  });
  void registration.update();
  showOfflineBanner(!navigator.onLine, options);
  window.addEventListener('online', () => showOfflineBanner(false, options));
  window.addEventListener('offline', () => showOfflineBanner(true, options));
  return registration;
}
