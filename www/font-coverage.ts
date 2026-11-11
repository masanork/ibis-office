// SPDX-License-Identifier: AGPL-3.0-only
/** Read the actual bootstrap font's Unicode cmap once. No extra font engine or
 * network lookup is needed to skip characters already available at startup. */
export type FontCoverage = (grapheme: string) => boolean;

export function fontCoverage(bytes: Uint8Array): FontCoverage {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const u16 = (at: number) => view.getUint16(at);
  const u32 = (at: number) => view.getUint32(at);
  const u24 = (at: number) => u16(at) * 256 + view.getUint8(at + 2);
  let cmap = -1, length = 0;
  for (let i = 0; i < u16(4); i++) {
    const at = 12 + 16 * i;
    if (u32(at) === 0x636d6170) { cmap = u32(at + 8); length = u32(at + 12); }
  }
  if (cmap < 0 || cmap + length > bytes.length) throw new Error('Font has no valid Unicode cmap');
  let bmp = -1, wide = -1, variation = -1;
  for (let i = 0; i < u16(cmap + 2); i++) {
    const at = cmap + 4 + i * 8, platform = u16(at), encoding = u16(at + 2);
    if (platform !== 0 && !(platform === 3 && [1, 10].includes(encoding))) continue;
    const table = cmap + u32(at + 4);
    if (table < cmap || table + 2 > cmap + length) throw new Error('Invalid font cmap offset');
    if (u16(table) === 4) bmp = table;
    if (u16(table) === 12) wide = table;
    if (u16(table) === 14) variation = table;
  }
  const find = (count: number, compare: (index: number) => number) => {
    let low = 0, high = count - 1;
    while (low <= high) {
      const mid = (low + high) >>> 1, order = compare(mid);
      if (!order) return mid;
      if (order < 0) high = mid - 1; else low = mid + 1;
    }
    return -1;
  };
  const base = (cp: number): boolean => {
    if (wide >= 0) {
      const i = find(u32(wide + 12), index => {
        const at = wide + 16 + index * 12;
        return cp < u32(at) ? -1 : cp > u32(at + 4) ? 1 : 0;
      });
      if (i >= 0) { const at = wide + 16 + i * 12; return u32(at + 8) + cp - u32(at) !== 0; }
    }
    if (cp > 0xffff || bmp < 0) return false;
    const count = u16(bmp + 6) / 2, ends = bmp + 14, starts = ends + 2 * count + 2;
    const i = find(count, index => cp < u16(starts + index * 2) ? -1 : cp > u16(ends + index * 2) ? 1 : 0);
    if (i < 0) return false;
    const delta = u16(starts + count * 2 + i * 2), offsetAt = starts + count * 4 + i * 2, offset = u16(offsetAt);
    const gid = offset ? u16(offsetAt + offset + 2 * (cp - u16(starts + i * 2))) : cp;
    return gid !== 0 && ((gid + delta) & 0xffff) !== 0;
  };
  const variant = (cp: number, vs: number): boolean => {
    if (variation < 0) return (vs === 0xfe0e || vs === 0xfe0f) && base(cp);
    const i = find(u32(variation + 6), index => vs - u24(variation + 10 + index * 11));
    if (i < 0) return (vs === 0xfe0e || vs === 0xfe0f) && base(cp);
    const record = variation + 10 + i * 11, nondefault = u32(record + 7), defaults = u32(record + 3);
    if (nondefault) {
      const at = variation + nondefault;
      const j = find(u32(at), index => cp - u24(at + 4 + index * 5));
      if (j >= 0) return u16(at + 7 + j * 5) !== 0;
    }
    if (defaults) {
      const at = variation + defaults;
      return find(u32(at), index => {
        const start = u24(at + 4 + index * 4), end = start + view.getUint8(at + 7 + index * 4);
        return cp < start ? -1 : cp > end ? 1 : 0;
      }) >= 0 && base(cp);
    }
    return false;
  };
  return grapheme => {
    try {
      const chars = [...grapheme];
      for (let i = 0; i < chars.length; i++) {
        if (/^[\p{Cc}\p{Cf}\s]$/u.test(chars[i])) continue;
        const cp = chars[i].codePointAt(0)!, vs = chars[i + 1]?.codePointAt(0) ?? 0;
        if ((vs >= 0xfe00 && vs <= 0xfe0f) || (vs >= 0xe0100 && vs <= 0xe01ef)) {
          if (!variant(cp, vs)) return false;
          i++;
        } else if (!base(cp)) return false;
      }
      return true;
    } catch { return false; }
  };
}
