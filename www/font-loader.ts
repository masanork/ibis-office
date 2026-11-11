// SPDX-License-Identifier: AGPL-3.0-only
/** Fonts are loaded before GPUI creates its first layout and font-name cache. */
export interface WebFonts {
  data: Uint8Array[];
  aliases: Record<string, string>;
  defaultFamily: string;
  uiFamily: string;
  browserFallback: boolean;
  dynamic?: { endpoint: string; font: string; strategy?: 'faces' };
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
  const uiFamily = manifest.uiFamily === undefined ? defaultFamily : name(manifest.uiFamily);
  const aliases = new Map<string, string>();
  const entries = manifest.fonts.map(value => {
    const font = object(value);
    const family = name(font.family);
    const resolveURL = (value: unknown): URL => {
      const source = new URL(name(value), url);
      if (!['http:', 'https:'].includes(source.protocol)) throw new Error('Invalid font URL');
      return source;
    };
    let parts: { source: URL; size: number; sha256: string }[] | undefined;
    let source: URL | undefined;
    if (font.parts !== undefined) {
      if (font.url !== undefined || !Array.isArray(font.parts) || font.parts.length === 0 || !Number.isSafeInteger(font.size) || (font.size as number) <= 0) throw new Error('Invalid font parts');
      parts = font.parts.map(value => {
        const part = object(value);
        if (!Number.isSafeInteger(part.size) || (part.size as number) <= 0 || (part.size as number) > 20 * 1024 * 1024 || typeof part.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(part.sha256)) throw new Error('Invalid font part');
        return { source: resolveURL(part.url), size: part.size as number, sha256: part.sha256 };
      });
      if (parts.reduce((sum, part) => sum + part.size, 0) !== font.size) throw new Error('Invalid font size');
    } else {
      source = resolveURL(font.url);
    }
    if (typeof font.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(font.sha256)) throw new Error('Missing font SHA-256');
    if (!Array.isArray(font.aliases)) throw new Error('Invalid font aliases');
    for (const alias of [family, ...font.aliases.map(name)]) {
      if (aliases.has(alias) && aliases.get(alias) !== family) throw new Error(`Conflicting font alias: ${alias}`);
      aliases.set(alias, family);
    }
    return { source, parts, family, sha256: font.sha256 };
  });
  if (!entries.some(font => font.family === defaultFamily)) throw new Error('Unknown default font family');
  if (!entries.some(font => font.family === uiFamily)) throw new Error('Unknown UI font family');
  const data = await Promise.all(entries.map(async font => {
    const sha256 = async (bytes: Uint8Array): Promise<string> => {
      const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes as Uint8Array<ArrayBuffer>));
      return [...hash].map(byte => byte.toString(16).padStart(2, '0')).join('');
    };
    const fetchBytes = async (source: URL): Promise<Uint8Array> => {
      const result = await fetcher(source);
      if (!result.ok) throw new Error(`Font ${font.family}: HTTP ${result.status}`);
      return new Uint8Array(await result.arrayBuffer());
    };
    let bytes: Uint8Array;
    if (font.parts) {
      const parts = await Promise.all(font.parts.map(async part => {
        const data = await fetchBytes(part.source);
        if (data.byteLength !== part.size || await sha256(data) !== part.sha256) throw new Error(`Font ${font.family}: invalid part`);
        return data;
      }));
      bytes = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
      let offset = 0;
      for (const part of parts) { bytes.set(part, offset); offset += part.length; }
    } else {
      bytes = await fetchBytes(font.source!);
    }
    const signature = bytes.byteLength >= 12 ? new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(0) : 0;
    if (signature !== 0x00010000 && signature !== 0x4f54544f) throw new Error(`Font ${font.family}: expected TTF/OTF`);
    if (await sha256(bytes) !== font.sha256) throw new Error(`Font ${font.family}: SHA-256 mismatch`);
    return bytes;
  }));
  let dynamic: WebFonts['dynamic'];
  if (manifest.dynamic !== undefined) {
    const settings = object(manifest.dynamic);
    const endpoint = new URL(name(settings.endpoint), url);
    const font = name(settings.font);
    if (!['http:', 'https:'].includes(endpoint.protocol) || !/^[A-Za-z][A-Za-z0-9]+$/.test(font) || manifest.browserFallback) {
      throw new Error('Invalid dynamic font configuration');
    }
    if (settings.strategy !== undefined && settings.strategy !== 'faces') throw new Error('Invalid dynamic strategy');
    dynamic = { endpoint: endpoint.href, font, ...(settings.strategy === 'faces' && { strategy: 'faces' as const }) };
  }
  return { data, aliases: Object.fromEntries(aliases), defaultFamily, uiFamily, browserFallback: manifest.browserFallback, dynamic };
}
