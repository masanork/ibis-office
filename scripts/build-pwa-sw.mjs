#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// Build www/sw.js from www/sw.template.js and the hashed assets of this build.
import { readFile, writeFile, stat } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readdirSync } from 'node:fs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const www = join(root, 'www');

function walk(dir, acc = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) walk(path, acc);
    else acc.push(path);
  }
  return acc;
}

function toUrl(path) {
  const rel = relative(www, path).split('\\').join('/');
  return './' + rel;
}

async function fileBytes(path) {
  try {
    return (await stat(path)).size;
  } catch {
    return 0;
  }
}

const shell = [
  './',
  './index.html',
  './editor.html',
  './offline.html',
  './manifest.webmanifest',
  './favicon.svg',
  './favicon.ico',
  './boot.js',
  './brand.js',
  './main.js',
  './build-info.js',
  './wasm-loader.js',
  './chunked-fetch.js',
  './host-loader.js',
  './ibisoffice-host.js',
  './subset-fetch.js',
  './heic-worker.js',
  './sorane-start.js',
  './sorane-github-vault.js',
  './pwa-register.js',
  './local-file.js',
  './pwa-sw-logic.js',
  './license.md',
  './pwa/icon-192.png',
  './pwa/icon-512.png',
  './pwa/apple-touch-icon.png',
  './loader-manifest.json',
  './pkg/ibis_office_app.js',
  './pkg/ibis_office_app_bg.manifest.json',
  './pkg/heic/ibis_heic_web.js',
  './pkg/heic/ibis_heic_web_bg.wasm',
];

const urls = new Set(shell);
let bytes = 0;

const manifestPath = join(www, 'pkg/ibis_office_app_bg.manifest.json');
const wasmManifest = JSON.parse(await readFile(manifestPath, 'utf8'));
if (wasmManifest.single?.sha256) {
  // The compressed singleton is enough; the four uncompressed chunks are ~70 MiB more.
  urls.add(`./pkg/wasm-br.${wasmManifest.single.sha256}.bin`);
  urls.add(`./pkg/br/${wasmManifest.single.sha256}.wasm`);
} else {
  for (const chunk of wasmManifest.chunks ?? []) {
    urls.add('./pkg/' + chunk.file);
  }
}

for (const snippet of walk(join(www, 'pkg/snippets'))) {
  if (snippet.endsWith('.js')) urls.add(toUrl(snippet));
}

const loaderPath = join(www, 'loader-manifest.json');
try {
  const loader = JSON.parse(await readFile(loaderPath, 'utf8'));
  for (const file of loader.files ?? []) {
    if (file.file) urls.add('./' + String(file.file).replace(/^\.\//, ''));
  }
  for (const font of loader.subsets?.fonts ?? []) {
    if (font.licenseFile) urls.add('./' + String(font.licenseFile).replace(/^\.\//, ''));
    for (const block of font.blocks ?? []) {
      if (block.file) urls.add('./' + String(block.file).replace(/^\.\//, ''));
    }
  }
} catch {
  // Fonts are optional when the subsetter did not run.
}

const existing = [];
for (const url of urls) {
  if (url.includes('/pkg/br/')) {
    existing.push(url);
    continue;
  }
  const path = join(www, url.replace(/^\.\//, ''));
  const size = await fileBytes(path);
  if (size > 0 || url === './') {
    existing.push(url);
    bytes += size;
  }
}
existing.sort();

const revisionSource = JSON.stringify({
  wasm: wasmManifest.sha256,
  urls: existing.filter((url) => !url.includes('/pkg/br/')),
});
const revision = createHash('sha256').update(revisionSource).digest('hex').slice(0, 16);
const precache = {
  version: '1',
  revision,
  urls: existing,
  bytes,
  wasm: wasmManifest.sha256,
};
const json = JSON.stringify(precache);
const template = await readFile(join(www, 'sw.template.js'), 'utf8');
if (!template.includes('/* __PRECACHE_JSON__ */')) {
  throw new Error('www/sw.template.js is missing the precache marker');
}
const sw = template.replace(
  '/* __PRECACHE_JSON__ */',
  `self.__SORANE_PRECACHE = ${json};`,
);
await writeFile(join(www, 'sw.js'), sw);
await writeFile(join(www, 'pwa-precache.json'), JSON.stringify(precache, null, 2) + '\n');
const mb = (bytes / (1024 * 1024)).toFixed(1);
console.log(`PWA precache ${existing.length} urls, ${mb} MiB on disk (revision ${revision})`);
