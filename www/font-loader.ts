// SPDX-License-Identifier: AGPL-3.0-only
/** Fonts are loaded before GPUI creates its first layout and font-name cache. */
export interface WebFonts {
  data: Uint8Array[];
  aliases: Record<string, string>;
  defaultFamily: string;
  browserFallback: boolean;
}

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid font manifest object');
  return value as Record<string, unknown>;
}

function name(value: unknown): string {
  if (typeof value !== 'string' || !value.trim()) throw new Error('Invalid font manifest name');
  return value;
}

export async function loadWebFonts(url: URL, fetcher: typeof fetch = fetch): Promise<WebFonts | null> {
  const response = await fetcher(url, { cache: 'no-cache' });
  // An absent manifest preserves the existing browser-font behavior. Any other
  // failure is visible: a configured font must never silently change typeface.
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Font manifest: HTTP ${response.status}`);
  const manifest = object(await response.json());
  if (manifest.version !== 1 || !Array.isArray(manifest.fonts) || manifest.fonts.length === 0) throw new Error('Unsupported font manifest');
  if (typeof manifest.browserFallback !== 'boolean') throw new Error('Invalid browserFallback');
  const defaultFamily = name(manifest.defaultFamily);
  const aliases = new Map<string, string>();
  const entries = manifest.fonts.map(value => {
    const font = object(value);
    const family = name(font.family);
    const source = new URL(name(font.url), url);
    if (!['http:', 'https:'].includes(source.protocol)) throw new Error('Invalid font URL');
    if (typeof font.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(font.sha256)) throw new Error('Missing font SHA-256');
    if (!Array.isArray(font.aliases)) throw new Error('Invalid font aliases');
    for (const alias of [family, ...font.aliases.map(name)]) {
      if (aliases.has(alias) && aliases.get(alias) !== family) throw new Error(`Conflicting font alias: ${alias}`);
      aliases.set(alias, family);
    }
    return { source, family, sha256: font.sha256 };
  });
  if (!entries.some(font => font.family === defaultFamily)) throw new Error('Unknown default font family');
  const data = await Promise.all(entries.map(async font => {
    const result = await fetcher(font.source);
    if (!result.ok) throw new Error(`Font ${font.family}: HTTP ${result.status}`);
    const buffer = await result.arrayBuffer();
    const signature = buffer.byteLength >= 12 ? new DataView(buffer).getUint32(0) : 0;
    if (signature !== 0x00010000 && signature !== 0x4f54544f) throw new Error(`Font ${font.family}: expected TTF/OTF`);
    const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', buffer));
    const hex = [...hash].map(byte => byte.toString(16).padStart(2, '0')).join('');
    if (hex !== font.sha256) throw new Error(`Font ${font.family}: SHA-256 mismatch`);
    return new Uint8Array(buffer);
  }));
  return { data, aliases: Object.fromEntries(aliases), defaultFamily, browserFallback: manifest.browserFallback };
}
