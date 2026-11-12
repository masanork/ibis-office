// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
// Boots the wasm editor. `pkg/` is produced by scripts/build-web.sh.
import init, { configure_fonts, enable_dynamic_fonts, enable_face_fonts, add_dynamic_font, add_face_font, run, set_embed_options } from './pkg/ibis_office_app.js';
import { loadWebFonts } from './font-loader.js';
import { loadWasm } from './wasm-loader.js';
import { DynamicFonts, FaceSubsetProvider } from './dynamic-fonts.js';
import { fontCoverage } from './font-coverage.js';

/** A file chosen by the user or loaded at startup. */
interface PickedFile {
  name: string;
  bytes: Uint8Array;
}

/**
 * How a page that embeds the editor (a cloud service's preview) sets it up.
 * Read before the editor starts, from `window.ibisOfficeOptions` (set by a
 * script before main.js) over the URL's parameters. See
 * publish/doc/web-embedding.md.
 */
interface EmbedOptions {
  /** Show 閉じる at the top right (`?close=1`). Default: not shown. */
  close?: boolean;
  /** Show ダウンロード, left of it (`?download=1`). Default: not shown. */
  download?: boolean;
  /** Open in read-only viewing mode (`?view=1`). Default: false (editable). */
  view?: boolean;
  /** Show the menu bar (`?menuBar=1`, `?menuBar=0` hides it). Default: true (including when `view=1`). */
  menuBar?: boolean;
  /** Show the toolbar (`?toolBar=1`, `?toolBar=0` hides it). Default: true (including when `view=1`). */
  toolBar?: boolean;
  /**
   * The origin the parent window is sent messages for (`?origin=https://app.example.com`),
   * when the editor is in an iframe. Without it the referrer's origin is used;
   * without that nothing is posted to the parent. `'*'` (any origin) only when
   * given explicitly.
   */
  targetOrigin?: string;
  /** Where the 'ibisoffice:close' and 'ibisoffice:download' events go (JavaScript only). Default: window. */
  eventTarget?: EventTarget;
}

declare global {
  interface Window {
    /** Called from Rust for ファイル → 開く. */
    ibisOfficePickFile: () => Promise<PickedFile | null>;
    /** Awaited by Rust after parsing the complete model, before layout. */
    ibisOfficePreloadFonts: (text: string) => Promise<void>;
    /** Download a locally generated DOCX without sending the document to a server. */
    ibisOfficeDownloadDocx: (name: string, bytes: Uint8Array) => void;
    /** Download locally generated HTML without sending the document to a server. */
    ibisOfficeDownloadHtml: (name: string, bytes: Uint8Array) => void;
    /** Called from Rust when the close or the download button is pressed. */
    ibisOfficeEmbedAction: (kind: string, name: string, bytes?: Uint8Array) => void;
    /** Set by the embedding page before main.js. */
    ibisOfficeOptions?: EmbedOptions;
  }
}

/** `?name=1` / `?name=true` / `?name` is on; `0`, `false` is off; absent is `undefined`. */
function flag(params: URLSearchParams, name: string): boolean | undefined {
  if (!params.has(name)) {
    return undefined;
  }
  const value = (params.get(name) ?? '').toLowerCase();
  return !(value === '0' || value === 'false' || value === 'no' || value === 'off');
}

/** The embedding options: JavaScript's over the URL's, and everything off by default. */
function embedOptions(): Required<Omit<EmbedOptions, 'eventTarget'>> & Pick<EmbedOptions, 'eventTarget'> {
  const params = new URLSearchParams(location.search);
  const js = window.ibisOfficeOptions ?? {};
  const view = js.view ?? flag(params, 'view') ?? false;
  return {
    close: js.close ?? flag(params, 'close') ?? false,
    download: js.download ?? flag(params, 'download') ?? false,
    view,
    menuBar: js.menuBar ?? flag(params, 'menuBar') ?? flag(params, 'menubar') ?? true,
    toolBar: js.toolBar ?? flag(params, 'toolBar') ?? flag(params, 'toolbar') ?? true,
    targetOrigin: js.targetOrigin ?? params.get('origin') ?? '',
    eventTarget: js.eventTarget,
  };
}

/** The origin messages to the parent window go to, or `null` for nowhere. */
function parentOrigin(option: string): string | null {
  if (option) {
    return option;
  }
  try {
    return document.referrer ? new URL(document.referrer).origin : null;
  } catch {
    return null;
  }
}

/**
 * Called from Rust when the close or the download button is pressed: the
 * page that embeds the editor gets a CustomEvent ('ibisoffice:close', or
 * 'ibisoffice:download' with `{ name, bytes, blob }`) on `window` (or the
 * `eventTarget` given), and, in an iframe, a message to the parent window
 * (`{ source: 'ibisoffice', type: 'close' | 'download', name, bytes }`).
 */
function embedAction(kind: string, name: string, bytes?: Uint8Array): void {
  const options = embedOptions();
  const detail = kind === 'download' && bytes ? { name, bytes, blob: new Blob([bytes as BlobPart]) } : {};
  (options.eventTarget ?? window).dispatchEvent(new CustomEvent(`ibisoffice:${kind}`, { detail }));
  if (window.parent !== window) {
    const origin = parentOrigin(options.targetOrigin);
    if (origin === null) {
      console.warn('ibisOffice: no target origin for the parent window (set ?origin=… or ibisOfficeOptions.targetOrigin); nothing was posted to it.');
      return;
    }
    // The parent gets its own copy of the bytes (transferred), so the event's listeners keep theirs.
    if (kind === 'download' && bytes) {
      const copy = bytes.slice().buffer;
      window.parent.postMessage({ source: 'ibisoffice', type: kind, name, bytes: copy }, origin, [copy]);
    } else {
      window.parent.postMessage({ source: 'ibisoffice', type: kind }, origin);
    }
  }
}

/** Show the browser's file picker. Resolves to `null` when cancelled. */
function pickFile(): Promise<PickedFile | null> {
  return new Promise((resolve, reject) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.jtd,.jtt,.jttc,.pptx,.xlsx,.docx,.ppt,.xls,.doc,application/vnd.openxmlformats-officedocument.presentationml.presentation,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,application/vnd.openxmlformats-officedocument.wordprocessingml.document';
    input.addEventListener('change', () => {
      const file = input.files?.[0];
      if (!file) {
        resolve(null);
        return;
      }
      file.arrayBuffer().then((buffer) => resolve({ name: file.name, bytes: new Uint8Array(buffer) }), reject);
    });
    input.addEventListener('cancel', () => resolve(null));
    input.click();
  });
}

/**
 * The file the playground page picked (`?local=1`). It is handed over through
 * the browser's own IndexedDB, never a server, and deleted once read.
 */
function localFile(): Promise<PickedFile | null> {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open('ibisoffice-playground', 1);
    open.onupgradeneeded = () => open.result.createObjectStore('files');
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const tx = open.result.transaction('files', 'readwrite');
      const files = tx.objectStore('files');
      const get = files.get('picked');
      get.onsuccess = () => {
        files.delete('picked');
        resolve((get.result as PickedFile | undefined) ?? null);
      };
      get.onerror = () => reject(get.error);
    };
  });
}

/** The document named by `?open=<url>`, fetched from this site, or the one the playground page picked. */
async function initialFile(): Promise<PickedFile | null> {
  if (new URLSearchParams(location.search).has('local')) {
    return localFile();
  }
  const url = new URLSearchParams(location.search).get('open');
  if (!url) {
    return null;
  }
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`${url}: HTTP ${response.status}`);
  }
  const path = new URL(url, location.href).pathname;
  const name = decodeURIComponent(path.slice(path.lastIndexOf('/') + 1));
  return { name, bytes: new Uint8Array(await response.arrayBuffer()) };
}

const status = document.getElementById('status');
const statusText = document.getElementById('status-text');
const progress = document.getElementById('progress');
const progressFill = progress?.firstElementChild as HTMLElement | null;

/** Download and verify the module before starting wasm-bindgen. */
async function fetchWasm(): Promise<Response> {
  const response = await loadWasm(new URL('./pkg/ibis_office_app_bg.manifest.json', import.meta.url), (loaded, total) => {
    if (progress) progress.style.display = 'block';
    if (progressFill) progressFill.style.width = `${Math.min(100, (loaded / total) * 100)}%`;
  });
  if (progressFill) {
    progressFill.style.transition = 'none';
    progressFill.style.width = '100%';
  }
  progress?.classList.add('starting');
  if (statusText) statusText.textContent = ja ? 'Sorane Docs を起動しています…' : 'Starting Sorane Docs…';
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  return response;
}

/** Japanese when the viewer chose it on the playground page, or when the browser prefers it. */
function prefersJapanese(): boolean {
  let saved: string | null = null;
  try {
    saved = localStorage.getItem('ibisoffice-lang');
  } catch {
    // Storage may be blocked; fall back to the browser's language.
  }
  return (saved ?? navigator.language).toLowerCase().startsWith('ja');
}

const ja = prefersJapanese();

function showStatus(text: string): void {
  if (statusText) {
    statusText.textContent = text;
  }
  if (progress) {
    progress.style.display = 'none';
  }
}

window.ibisOfficeDownloadDocx = (name, bytes) => {
  downloadDocument(name, bytes, 'application/vnd.openxmlformats-officedocument.wordprocessingml.document');
};
window.ibisOfficeDownloadHtml = (name, bytes) => {
  downloadDocument(name, bytes, 'text/html;charset=utf-8');
};

function downloadDocument(name: string, bytes: Uint8Array, type: string): void {
  const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type }));
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

window.ibisOfficePickFile = pickFile;
window.ibisOfficeEmbedAction = embedAction;

if (!('gpu' in navigator)) {
  showStatus(ja
    ? 'このブラウザは WebGPU に対応していません。最新の Chrome、Edge、Safari をお使いください。'
    : 'This browser does not support WebGPU. Please use the latest Chrome, Edge or Safari.');
} else {
  try {
    const [, file, fonts] = await Promise.all([
      init({ module_or_path: fetchWasm() }), initialFile(),
      loadWebFonts(new URL('./fonts.json', import.meta.url)),
    ]);
    if (fonts) configure_fonts(fonts.data, fonts.aliases, fonts.defaultFamily, fonts.browserFallback, fonts.uiFamily);
    if (fonts?.dynamic) {
      if (fonts.dynamic.strategy === 'faces') enable_face_fonts();
      const provider = fonts.dynamic.strategy === 'faces'
        ? new FaceSubsetProvider(fonts.dynamic.endpoint, add_face_font)
        : new DynamicFonts(fonts.dynamic, add_dynamic_font);
      for (let i = 0; i < fonts.data.length; i++) {
        const covers = fontCoverage(fonts.data[i]);
        if (provider instanceof FaceSubsetProvider) {
          const family = fonts.families[i];
          const gothic = fonts.aliases['Noto Sans CJK JP'] ?? fonts.uiFamily;
          if (family === fonts.defaultFamily) provider.seed('ipamjm', covers);
          if (family === gothic) provider.seed('NotoSansCJKJP', covers);
          if (family !== fonts.defaultFamily && family !== gothic) {
            provider.seed('ipamjm', covers); provider.seed('NotoSansCJKJP', covers);
          }
        } else provider.seed(covers);
      }
      Object.assign(window, {
        ibisOfficeMissingGlyphs: (glyphs: string[]) => provider.request(glyphs),
        ibisOfficePreloadFonts: async (text: string) => {
          showStatus(ja ? '文書のフォントを読み込んでいます…' : 'Loading document fonts…');
          await provider.preload(text);
        },
      });
      enable_dynamic_fonts();
    }
    // Icons load from <base>/assets/icons/*.svg, next to this page (the site
    // may live under a path, as on GitHub Pages). The base must be absolute:
    // the icon loader fetches with reqwest, which rejects relative URLs.
    const base = new URL('.', location.href).href.replace(/\/$/, '');
    const embed = embedOptions();
    set_embed_options(embed.close, embed.download, embed.view, embed.menuBar, embed.toolBar);
    await run(base, file?.name, file?.bytes);
    status?.remove();
  } catch (error) {
    console.error(error);
    showStatus(ja ? `起動できませんでした: ${error}` : `Could not start: ${error}`);
  }
}
