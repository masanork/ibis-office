// SPDX-License-Identifier: AGPL-3.0-only
// Small SFNT helpers for the build-time font pipeline. Rendering stays in GPUI.
import { Buffer } from 'node:buffer';

export function tables(bytes) {
  const data = Buffer.from(bytes);
  if (data.length < 12 || ![0x00010000, 0x4f54544f].includes(data.readUInt32BE(0))) {
    throw new Error('Expected a single TTF/OTF font (not TTC, WOFF or WOFF2)');
  }
  const result = new Map();
  const count = data.readUInt16BE(4);
  if (12 + count * 16 > data.length) throw new Error('Truncated SFNT directory');
  for (let i = 0; i < count; i++) {
    const at = 12 + i * 16;
    const tag = data.toString('ascii', at, at + 4);
    const offset = data.readUInt32BE(at + 8);
    const length = data.readUInt32BE(at + 12);
    if (offset + length > data.length) throw new Error(`Truncated ${tag} table`);
    result.set(tag, data.subarray(offset, offset + length));
  }
  return result;
}

function nameRecords(data) {
  if (!data || data.length < 6) throw new Error('Missing font name table');
  const count = data.readUInt16BE(2);
  const strings = data.readUInt16BE(4);
  if (6 + count * 12 > data.length) throw new Error('Truncated name records');
  return Array.from({ length: count }, (_, i) => {
    const at = 6 + i * 12;
    const header = Buffer.from(data.subarray(at, at + 12));
    const start = strings + header.readUInt16BE(10);
    const end = start + header.readUInt16BE(8);
    if (end > data.length) throw new Error('Truncated font name');
    return { header, value: data.subarray(start, end) };
  });
}

function unicodeName(record) {
  const platform = record.header.readUInt16BE(0);
  if (platform !== 0 && platform !== 3) return null;
  return Buffer.from(record.value).swap16().toString('utf16le');
}

export function fontFamily(bytes) {
  const records = nameRecords(tables(bytes).get('name'));
  for (const id of [16, 1]) {
    const matches = records.filter(r => r.header.readUInt16BE(6) === id);
    matches.sort((a, b) => Number(b.header.readUInt16BE(4) === 0x409) - Number(a.header.readUInt16BE(4) === 0x409));
    for (const record of matches) {
      const name = unicodeName(record);
      if (name) return name;
    }
  }
  throw new Error('Font has no Unicode family name');
}

export function checksum(bytes) {
  let sum = 0;
  for (let i = 0; i < bytes.length; i += 4) {
    let word = 0;
    for (let j = 0; j < 4; j++) word = (word * 256 + (bytes[i + j] ?? 0)) >>> 0;
    sum = (sum + word) >>> 0;
  }
  return sum;
}

/** The engine names every subset "IBS Subset". Give separate faces distinct families,
 * preserving copyright/license records and every non-name table (including cmap14).
 */
export function renameSubset(bytes, family) {
  const source = tables(bytes);
  const records = nameRecords(source.get('name'));
  const style = records.find(r => r.header.readUInt16BE(6) === 17 && unicodeName(r))
    ?? records.find(r => r.header.readUInt16BE(6) === 2 && unicodeName(r));
  const subfamily = style ? unicodeName(style) : 'Regular';
  const ps = `${family}-${subfamily}`.replace(/[^a-zA-Z0-9-]/g, '').slice(0, 63);
  const replaced = new Set([1, 3, 4, 6, 16, 18, 20, 21, 25]);
  const kept = records.filter(r => !replaced.has(r.header.readUInt16BE(6)));
  // Name format 1 language-tag references need a format 1 table; retain the
  // ordinary records and use English for the newly generated names.
  const output = kept.filter(r => r.header.readUInt16BE(4) < 0x8000);
  for (const [id, text] of [[1, family], [3, ps], [4, `${family} ${subfamily}`], [6, ps], [16, family], [21, family]]) {
    const value = Buffer.from(text, 'utf16le').swap16();
    const header = Buffer.alloc(12);
    [3, 1, 0x409, id, value.length, 0].forEach((v, i) => header.writeUInt16BE(v, i * 2));
    output.push({ header, value });
  }
  const stringOffset = 6 + output.length * 12;
  const name = Buffer.alloc(stringOffset + output.reduce((n, r) => n + r.value.length, 0));
  name.writeUInt16BE(output.length, 2);
  name.writeUInt16BE(stringOffset, 4);
  let at = stringOffset;
  output.forEach((record, i) => {
    record.header.copy(name, 6 + i * 12);
    name.writeUInt16BE(at - stringOffset, 6 + i * 12 + 10);
    record.value.copy(name, at);
    at += record.value.length;
  });
  source.set('name', name);
  const head = Buffer.from(source.get('head') ?? []);
  if (head.length < 12) throw new Error('Missing head table');
  head.writeUInt32BE(0, 8);
  source.set('head', head);
  // A digital signature no longer describes the modified font.
  source.delete('DSIG');
  const entries = [...source].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0);
  let offset = 12 + entries.length * 16;
  const result = Buffer.alloc(offset + entries.reduce((n, [, data]) => n + ((data.length + 3) & ~3), 0));
  Buffer.from(bytes).copy(result, 0, 0, 4);
  result.writeUInt16BE(entries.length, 4);
  const power = 2 ** Math.floor(Math.log2(entries.length));
  result.writeUInt16BE(power * 16, 6);
  result.writeUInt16BE(Math.log2(power), 8);
  result.writeUInt16BE(entries.length * 16 - power * 16, 10);
  let headOffset = 0;
  entries.forEach(([tag, data], i) => {
    const record = 12 + i * 16;
    result.write(tag, record, 4, 'ascii');
    result.writeUInt32BE(checksum(data), record + 4);
    result.writeUInt32BE(offset, record + 8);
    result.writeUInt32BE(data.length, record + 12);
    data.copy(result, offset);
    if (tag === 'head') headOffset = offset;
    offset += (data.length + 3) & ~3;
  });
  result.writeUInt32BE((0xb1b0afba - checksum(result)) >>> 0, headOffset + 8);
  return result;
}
