// SPDX-License-Identifier: AGPL-3.0-only
// Uses gjs's existing WASM subsetter at build time; no gjs service is required.
import { readFile, writeFile, mkdir, realpath } from 'node:fs/promises';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';
import { fontFamily, renameSubset, tables } from './web-font-sfnt.mjs';

const subsetters = new Map();
async function subsetter(gjsDir) {
  const pkg = await realpath(join(gjsDir, 'crates/font-subsetter/pkg'));
  // wasm-bindgen's glue caches views of its instance's memory. Rebinding the
  // same glue module to a second instance leaves those views on the first one.
  if (!subsetters.has(pkg)) {
    const ready = (async () => {
      const bg = await import(pathToFileURL(join(pkg, 'font_subsetter_bg.js')).href);
      const wasm = await readFile(join(pkg, 'font_subsetter_bg.wasm'));
      const { instance } = await WebAssembly.instantiate(wasm, { './font_subsetter_bg.js': bg });
      bg.__wbg_set_wasm(instance.exports);
      instance.exports.__wbindgen_start?.();
      return bg;
    })();
    subsetters.set(pkg, ready);
    ready.catch(() => subsetters.delete(pkg));
  }
  return subsetters.get(pkg);
}

export async function buildWebFonts(configPath, { gjsDir = resolve('../gjs'), outDir = resolve('www') } = {}) {
  const config = JSON.parse(await readFile(configPath, 'utf8'));
  if (!Array.isArray(config.fonts) || config.fonts.length === 0) throw new Error('fonts must be a nonempty array');
  if (config.browserFallback !== undefined && typeof config.browserFallback !== 'boolean') throw new Error('browserFallback must be boolean');
  const base = dirname(resolve(configPath));
  const ids = new Map();
  const aliases = new Map();
  const artifacts = [];
  const fonts = [];
  let bg;
  const digest = data => createHash('sha256').update(data).digest('hex');
  for (const entry of config.fonts) {
    if (typeof entry.id !== 'string' || !/^[a-zA-Z0-9_-]+$/.test(entry.id) || ids.has(entry.id)) throw new Error('Each font needs a unique id using letters, digits, _ or -');
    if (typeof entry.path !== 'string') throw new Error(`${entry.id}: path is required`);
    let data = await readFile(resolve(base, entry.path));
    tables(data);
    const originalFamily = fontFamily(data);
    let family = originalFamily;
    if (entry.text !== undefined || entry.textFile !== undefined) {
      if (entry.text !== undefined && typeof entry.text !== 'string') throw new Error(`${entry.id}: text must be a string`);
      const text = (entry.text ?? '') + (entry.textFile === undefined ? '' : await readFile(resolve(base, entry.textFile), 'utf8'));
      if (!text.trim()) throw new Error(`${entry.id}: subset text is empty`);
      bg ??= await subsetter(resolve(gjsDir));
      if (!bg.font_has_glyphs(data, text.replace(/[\r\n\t]/g, ''))) throw new Error(`${entry.id}: source font lacks requested characters`);
      family = entry.family ?? `Ibis Subset ${entry.id}`;
      if (typeof family !== 'string' || !family.trim() || family.length > 100) throw new Error(`${entry.id}: invalid subset family`);
      data = renameSubset(bg.subset_font_raw(data, text), family);
    } else if (entry.family !== undefined) {
      throw new Error(`${entry.id}: family may only rename a subset`);
    }
    const sha256 = digest(data);
    const filename = `${entry.id}-${sha256}.otf`;
    const names = entry.aliases ?? [];
    if (!Array.isArray(names) || names.some(n => typeof n !== 'string' || !n.trim())) throw new Error(`${entry.id}: aliases must be nonempty strings`);
    for (const name of [family, originalFamily, ...names]) {
      if (aliases.has(name) && aliases.get(name) !== family) throw new Error(`Conflicting font alias: ${name}`);
      aliases.set(name, family);
    }
    ids.set(entry.id, family);
    const font = { url: `./assets/fonts/${filename}`, family, sha256, aliases: [...new Set([originalFamily, ...names])] };
    artifacts.push([filename, data]);
    if (entry.licenseFile !== undefined) {
      const license = await readFile(resolve(base, entry.licenseFile));
      const name = `${entry.id}-${digest(license)}.license.txt`;
      artifacts.push([name, license]);
      font.licenseUrl = `./assets/fonts/${name}`;
    }
    fonts.push(font);
  }
  const defaultFamily = ids.get(config.defaultFont ?? config.fonts[0].id);
  if (!defaultFamily) throw new Error('defaultFont must name a configured font id');
  // Validate everything before publishing the manifest. Old content-addressed
  // assets remain valid for tabs that still have an older manifest.
  const assetDir = join(outDir, 'assets/fonts');
  await mkdir(assetDir, { recursive: true });
  for (const [name, data] of artifacts) await writeFile(join(assetDir, name), data);
  const manifest = { version: 1, defaultFamily, browserFallback: config.browserFallback ?? false, fonts };
  await writeFile(join(outDir, 'fonts.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [config, gjsDir, outDir] = process.argv.slice(2);
  if (!config) {
    console.error('Usage: node scripts/build-web-fonts.mjs CONFIG.json [GJS_DIR] [WWW_DIR]');
    process.exitCode = 1;
  } else {
    try {
      const result = await buildWebFonts(resolve(config), { gjsDir, outDir });
      console.log(`Prepared ${result.fonts.length} font(s); default: ${result.defaultFamily}`);
    } catch (error) {
      console.error(error.message);
      process.exitCode = 1;
    }
  }
}
