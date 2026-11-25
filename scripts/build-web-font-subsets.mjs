#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// Build static font subset blocks (SIL OFL 1.1) with upstream `tools/ibis-font-subsetter`
// and write `loader-manifest.json` for `www/subset-fetch.ts`.
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export const DEFAULT_FONT_IDS = ['ACGJMincho', 'BIZUDGothic', 'BIZUDMincho'];

export const TYPEFACE_ALIASES = {
  ACGJMincho: [
    '追加文字行政事務標準明朝',
    'Add Character_GJMincho',
    'GJ明朝',
    'GJMincho',
  ],
  BIZUDGothic: [
    'BIZ UDゴシック',
    'BIZ UDGothic',
    'BIZ UDPゴシック',
    'BIZ UDPGothic',
    '游ゴシック',
    'Yu Gothic',
    'YuGothic',
    'MS Gothic',
    'ＭＳ ゴシック',
    'ＭＳゴシック',
    'ＭＳ Ｐゴシック',
    'ＭＳＰゴシック',
    'MS PGothic',
    'メイリオ',
    'Meiryo',
    'Meiryo UI',
    'Hiragino Sans',
    'Hiragino Kaku Gothic ProN',
    'ヒラギノ角ゴ ProN',
    'Noto Sans JP',
    'Noto Sans CJK JP',
    'IPAexゴシック',
    'IPAexGothic',
    'IPAゴシック',
    'IPAGothic',
  ],
  BIZUDMincho: [
    'BIZ UD明朝',
    'BIZ UDMincho',
    'BIZ UDP明朝',
    'BIZ UDPMincho',
    'BIZ UD明朝 Medium',
    'BIZ UDMincho Medium',
    'BIZ UDP明朝 Medium',
    'BIZ UDPMincho Medium',
    '游明朝',
    'Yu Mincho',
    'YuMincho',
    'MS Mincho',
    'ＭＳ 明朝',
    'ＭＳ明朝',
    'ＭＳ Ｐ明朝',
    'ＭＳＰ明朝',
    'MS PMincho',
    'Hiragino Mincho ProN',
    'ヒラギノ明朝 ProN',
    'Noto Serif JP',
    'Noto Serif CJK JP',
    'IPAex明朝',
    'IPAexMincho',
    'IPA明朝',
    'IPAMincho',
  ],
};

export function parseFontList(text) {
  const entries = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    entries.push(JSON.parse(line));
  }
  return entries;
}

export function readWasmVersion(versionText) {
  for (const raw of versionText.split(/\r?\n/)) {
    const [kind, ver] = raw.trim().split('\t');
    if (kind === 'wasm' && ver) return ver.trim();
  }
  throw new Error('wasm version not found in version.txt');
}

export function buildLoaderManifest(appVersion, fragments, prefix = 'fonts') {
  const cleanPrefix = prefix.replace(/^\/+|\/+$/g, '');
  const qualify = (rel) => (cleanPrefix ? `${cleanPrefix}/${rel.replace(/^\/+/, '')}` : rel.replace(/^\/+/, ''));
  return {
    schema: 1,
    app: appVersion,
    chunkSize: 0,
    encoding: 'none',
    files: [],
    subsets: {
      schema: 1,
      fonts: fragments.map((frag) => ({
        id: frag.id,
        typeface: frag.typeface,
        license: frag.license,
        licenseFile: qualify(frag.licenseFile),
        blocks: frag.blocks.map((block) => ({
          key: block.key,
          family: block.family,
          file: qualify(block.file),
          size: block.size,
          sha256: block.sha256,
          ranges: block.ranges,
          ...(block.boot ? { boot: true } : {}),
        })),
      })),
    },
  };
}

function sha256Hex(buf) {
  return createHash('sha256').update(buf).digest('hex');
}

async function ensureCachedFile(filePath, url, expectedSize, expectedSha256, allowDownload) {
  if (existsSync(filePath)) {
    const existing = await readFile(filePath);
    if ((!expectedSize || existing.byteLength === expectedSize) && (!expectedSha256 || sha256Hex(existing) === expectedSha256)) {
      return true;
    }
    if (!allowDownload) return false;
  } else if (!allowDownload) {
    return false;
  }
  if (!url) throw new Error(`Missing URL for ${filePath}`);
  console.log(`Downloading ${url} -> ${filePath}`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Failed to fetch ${url}: HTTP ${res.status}`);
  const data = Buffer.from(await res.arrayBuffer());
  if (expectedSize && data.byteLength !== expectedSize) {
    throw new Error(`${filePath}: size mismatch (got ${data.byteLength}, expected ${expectedSize})`);
  }
  if (expectedSha256 && sha256Hex(data) !== expectedSha256) {
    throw new Error(`${filePath}: SHA-256 mismatch (got ${sha256Hex(data)}, expected ${expectedSha256})`);
  }
  await mkdir(dirname(filePath), { recursive: true });
  await writeFile(filePath, data);
  return true;
}

function parseArgs(argv) {
  const opts = {
    out: resolve(ROOT, 'www'),
    cache: resolve(ROOT, 'data/fonts'),
    fontList: resolve(ROOT, 'docs/font-list.txt'),
    fonts: [...DEFAULT_FONT_IDS],
    chunk: '512KiB',
    bin: '',
    ifCached: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--out') opts.out = resolve(argv[++i]);
    else if (arg === '--cache') opts.cache = resolve(argv[++i]);
    else if (arg === '--font-list') opts.fontList = resolve(argv[++i]);
    else if (arg === '--fonts') opts.fonts = argv[++i].split(',').map((s) => s.trim()).filter(Boolean);
    else if (arg === '--chunk') opts.chunk = argv[++i];
    else if (arg === '--bin') opts.bin = resolve(argv[++i]);
    else if (arg === '--if-cached') opts.ifCached = true;
    else throw new Error(`Unknown option: ${arg}`);
  }
  return opts;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const listEntries = parseFontList(await readFile(opts.fontList, 'utf8'));
  const byId = new Map(listEntries.map((e) => [e.id, e]));
  const selected = [];
  for (const id of opts.fonts) {
    const entry = byId.get(id);
    if (!entry) throw new Error(`Font id not found in ${opts.fontList}: ${id}`);
    if (entry.license !== 'SIL OFL 1.1') {
      throw new Error(`Refusing to statically subset non-OFL font ${id} (${entry.license})`);
    }
    selected.push(entry);
  }

  const resolved = [];
  for (const entry of selected) {
    const fontPath = join(opts.cache, entry.file);
    const licensePath = join(opts.cache, `${entry.id}-OFL.txt`);
    const haveFont = await ensureCachedFile(fontPath, entry.url, entry.size, entry.sha256, !opts.ifCached);
    const haveLicense = await ensureCachedFile(licensePath, entry.license_url, 0, '', !opts.ifCached);
    if (!haveFont || !haveLicense) {
      if (opts.ifCached) {
        console.log(`Skipping static font subsets (${entry.id} not cached in ${opts.cache}; set IBIS_WEB_FONT_SUBSETS=1 to download).`);
        return;
      }
      throw new Error(`Missing font or license for ${entry.id}`);
    }
    resolved.push({ entry, fontPath, licensePath });
  }

  const fontsOut = join(opts.out, 'fonts');
  await rm(fontsOut, { recursive: true, force: true });
  await mkdir(fontsOut, { recursive: true });

  const targetDir = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(ROOT, 'target');
  let binPath = opts.bin;
  if (!binPath) {
    const buildRes = spawnSync('cargo', ['build', '--release', '--locked', '-p', 'ibis-font-block-subsetter', '--bin', 'ibis-font-block-subsetter'], {
      cwd: ROOT,
      stdio: 'inherit',
    });
    if (buildRes.status !== 0) process.exit(buildRes.status ?? 1);
    binPath = join(targetDir, 'release', process.platform === 'win32' ? 'ibis-font-block-subsetter.exe' : 'ibis-font-block-subsetter');
  }
  const fragments = [];
  for (const { entry, fontPath, licensePath } of resolved) {
    const typefaces = TYPEFACE_ALIASES[entry.id] ?? [entry.name_ja ?? entry.name_en, entry.name_en];
    const args = ['--font', fontPath, '--id', entry.id, '--license', licensePath, '--out', fontsOut, '--chunk', opts.chunk];
    for (const tf of typefaces) args.push('--typeface', tf);
    const res = spawnSync(binPath, args, { cwd: ROOT, stdio: 'inherit' });
    if (res.status !== 0) process.exit(res.status ?? 1);
    const fragment = JSON.parse(await readFile(join(fontsOut, entry.id, 'blocks.json'), 'utf8'));
    fragments.push(fragment);
  }

  const appVersion = readWasmVersion(await readFile(join(ROOT, 'version.txt'), 'utf8'));
  const manifest = buildLoaderManifest(appVersion, fragments, 'fonts');
  const manifestPath = join(opts.out, 'loader-manifest.json');
  await writeFile(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
  console.log(`Wrote ${manifestPath} (${fragments.length} OFL fonts, app ${appVersion})`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
