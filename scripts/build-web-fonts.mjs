// SPDX-License-Identifier: AGPL-3.0-only
// Uses the source-built ibisOffice subsetter; no external repository is required.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { fontFamily, renameSubset, tables } from './web-font-sfnt.mjs';
import { loadSubsetter } from './font-subsetter.mjs';
import { uiRepertoire } from './ui-font-repertoire.mjs';

export async function buildWebFonts(configPath, { outDir = resolve('www'), uiSources } = {}) {
  const config = JSON.parse(await readFile(configPath, 'utf8'));
  if (!Array.isArray(config.fonts) || config.fonts.length === 0) throw new Error('fonts must be a nonempty array');
  if (config.browserFallback !== undefined && typeof config.browserFallback !== 'boolean') throw new Error('browserFallback must be boolean');
  const base = dirname(resolve(configPath));
  const ids = new Map();
  const configuredIds = new Set();
  const aliases = new Map();
  const artifacts = [];
  const fonts = [];
  const remainingUi = new Set(config.fonts.some(entry => entry.uiResources === true) ? await uiRepertoire(uiSources) : []);
  let bg;
  const digest = data => createHash('sha256').update(data).digest('hex');
  for (const entry of config.fonts) {
    if (typeof entry.id !== 'string' || !/^[a-zA-Z0-9_-]+$/.test(entry.id) || configuredIds.has(entry.id)) throw new Error('Each font needs a unique id using letters, digits, _ or -');
    configuredIds.add(entry.id);
    if (typeof entry.path !== 'string') throw new Error(`${entry.id}: path is required`);
    let data = await readFile(resolve(base, entry.path));
    tables(data);
    const originalFamily = fontFamily(data);
    let family = originalFamily;
    if (entry.uiResources !== undefined && typeof entry.uiResources !== 'boolean') throw new Error(`${entry.id}: uiResources must be boolean`);
    if (entry.text !== undefined || entry.textFile !== undefined || entry.uiResources) {
      if (entry.text !== undefined && typeof entry.text !== 'string') throw new Error(`${entry.id}: text must be a string`);
      let text = (entry.text ?? '') + (entry.textFile === undefined ? '' : await readFile(resolve(base, entry.textFile), 'utf8'));
      bg ??= await loadSubsetter();
      if (entry.uiResources) {
        const engine = new bg.FontSubsetter(data.length);
        try {
          engine.append(data);
          for (const glyph of remainingUi) if (engine.has_glyphs(glyph)) {
            text += glyph;
            remainingUi.delete(glyph);
          }
        } finally { engine.free(); }
        // An optional symbol face can become unnecessary after a UI update.
        if (!text.trim()) continue;
      }
      if (!text.trim()) throw new Error(`${entry.id}: subset text is empty`);
      if (!bg.font_has_glyphs(data, text.replace(/[\r\n\t]/g, ''))) throw new Error(`${entry.id}: source font lacks requested characters`);
      family = entry.family ?? `Ibis Subset ${entry.id}`;
      if (typeof family !== 'string' || !family.trim() || family.length > 100) throw new Error(`${entry.id}: invalid subset family`);
      data = renameSubset(bg.subset_font_raw(data, text), family);
    } else if (entry.family !== undefined) {
      throw new Error(`${entry.id}: family may only rename a subset`);
    }
    const sha256 = digest(data);
    const names = entry.aliases ?? [];
    if (!Array.isArray(names) || names.some(n => typeof n !== 'string' || !n.trim())) throw new Error(`${entry.id}: aliases must be nonempty strings`);
    for (const name of [family, originalFamily, ...names]) {
      if (aliases.has(name) && aliases.get(name) !== family) throw new Error(`Conflicting font alias: ${name}`);
      aliases.set(name, family);
    }
    ids.set(entry.id, family);
    const font = { family, sha256, aliases: [...new Set([originalFamily, ...names])] };
    const chunkSize = 20 * 1024 * 1024;
    if (data.length > chunkSize) {
      font.size = data.length;
      font.parts = [];
      for (let offset = 0; offset < data.length; offset += chunkSize) {
        const part = data.subarray(offset, Math.min(offset + chunkSize, data.length));
        const partHash = digest(part);
        const filename = `${entry.id}-${partHash}.fontchunk`;
        font.parts.push({ url: `./assets/fonts/${filename}`, size: part.length, sha256: partHash });
        artifacts.push([filename, part]);
      }
    } else {
      const filename = `${entry.id}-${sha256}.otf`;
      font.url = `./assets/fonts/${filename}`;
      artifacts.push([filename, data]);
    }
    if (entry.licenseFile !== undefined) {
      const license = await readFile(resolve(base, entry.licenseFile));
      const name = `${entry.id}-${digest(license)}.license.txt`;
      artifacts.push([name, license]);
      font.licenseUrl = `./assets/fonts/${name}`;
    }
    fonts.push(font);
  }
  if (remainingUi.size) throw new Error('UI resources lack bootstrap glyphs: ' + [...remainingUi]
    .map(glyph => [...glyph].map(c => 'U+' + c.codePointAt(0).toString(16).toUpperCase()).join('+')).join(', '));
  if (config.dynamic !== undefined) {
    if (!config.dynamic || typeof config.dynamic.endpoint !== 'string' || !(/^(https?:\/\/|\/(?!\/))/.test(config.dynamic.endpoint))
        || typeof config.dynamic.font !== 'string' || !/^[A-Za-z][A-Za-z0-9]+$/.test(config.dynamic.font)) {
      throw new Error('dynamic requires an HTTP(S) or same-origin endpoint and a font ID');
    }
    if (config.browserFallback === true) throw new Error('dynamic fonts require browserFallback: false');
    if (config.dynamic.strategy !== undefined && config.dynamic.strategy !== 'faces') throw new Error('Invalid dynamic strategy');
  }
  const defaultFamily = ids.get(config.defaultFont ?? config.fonts[0].id);
  if (!defaultFamily) throw new Error('defaultFont must name a configured font id');
  const uiFamily = config.uiFont === undefined ? defaultFamily : ids.get(config.uiFont);
  if (!uiFamily) throw new Error('uiFont must name a configured font id');
  // Validate everything before publishing the manifest. Old content-addressed
  // assets remain valid for tabs that still have an older manifest.
  const assetDir = join(outDir, 'assets/fonts');
  await mkdir(assetDir, { recursive: true });
  for (const [name, data] of artifacts) await writeFile(join(assetDir, name), data);
  const manifest = { version: 1, defaultFamily, uiFamily, browserFallback: config.browserFallback ?? false, fonts, ...(config.dynamic && { dynamic: config.dynamic }) };
  await writeFile(join(outDir, 'fonts.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [config, outDir] = process.argv.slice(2);
  if (!config) {
    console.error('Usage: node scripts/build-web-fonts.mjs CONFIG.json [WWW_DIR]');
    process.exitCode = 1;
  } else {
    try {
      const result = await buildWebFonts(resolve(config), { outDir });
      console.log(`Prepared ${result.fonts.length} font(s); default: ${result.defaultFamily}`);
    } catch (error) {
      console.error(error.message);
      process.exitCode = 1;
    }
  }
}
