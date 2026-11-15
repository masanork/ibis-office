// SPDX-License-Identifier: AGPL-3.0-only
import { fail } from '../vendor/mikaki-oidc';

export const SAMPLE_MAX_FILE = 2 * 1024 * 1024;
export const USER_MAX_FILE = 32 * 1024 * 1024;
export const USER_MAX_VERSIONS = 10;
export const OFFICE_NAME = /^[A-Za-z0-9._-]{1,80}\.(docx|xlsx|pptx)$/i;

export async function boundedBody(request: Request, maxFile: number): Promise<Uint8Array> {
  const declared = Number(request.headers.get('content-length'));
  if (Number.isFinite(declared) && declared > maxFile) fail(413, 'file_too_large');
  if (!request.body) fail(400, 'missing_file');
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    length += value.length;
    if (length > maxFile) {
      await reader.cancel();
      fail(413, 'file_too_large');
    }
    chunks.push(value);
  }
  const body = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.length;
  }
  return body;
}

export function officeExtension(name: string): 'docx' | 'xlsx' | 'pptx' | null {
  const match = /\.(docx|xlsx|pptx)$/i.exec(name);
  return match ? (match[1].toLowerCase() as 'docx' | 'xlsx' | 'pptx') : null;
}

export function safeOfficeName(raw: string): string | null {
  const trimmed = raw.trim();
  if (!OFFICE_NAME.test(trimmed)) return null;
  const ext = officeExtension(trimmed);
  if (!ext) return null;
  const base = trimmed.slice(0, trimmed.length - ext.length - 1);
  if (base.length === 0 || base === '.' || base === '..') return null;
  return `${base}.${ext}`;
}

// Check the ZIP directory without expanding uploaded data. This is a bounded
// structural filter, not a full OOXML validator or malware scan.
export function looksLikeOffice(bytes: Uint8Array, name: string): boolean {
  if (bytes.length < 22) return false;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const u16 = (p: number) => view.getUint16(p, true),
    u32 = (p: number) => view.getUint32(p, true);
  let end = -1;
  for (let p = bytes.length - 22; p >= Math.max(0, bytes.length - 65557); p--) {
    if (u32(p) === 0x06054b50 && p + 22 + u16(p + 20) === bytes.length) {
      end = p;
      break;
    }
  }
  if (end < 0 || u16(end + 4) !== 0 || u16(end + 6) !== 0) return false;
  const count = u16(end + 10),
    size = u32(end + 12),
    start = u32(end + 16);
  if (count === 0xffff || size === 0xffffffff || start === 0xffffffff || start + size !== end)
    return false;
  const expected = name.endsWith('.docx')
    ? 'word/document.xml'
    : name.endsWith('.xlsx')
      ? 'xl/workbook.xml'
      : 'ppt/presentation.xml';
  let p = start,
    contentTypes = false,
    document = false;
  for (let i = 0; i < count; i++) {
    if (p + 46 > end || u32(p) !== 0x02014b50 || (u16(p + 8) & 1) !== 0) return false;
    const fileLength = u16(p + 28),
      extra = u16(p + 30),
      comment = u16(p + 32),
      next = p + 46 + fileLength + extra + comment;
    if (next > end) return false;
    const path = new TextDecoder().decode(bytes.subarray(p + 46, p + 46 + fileLength));
    contentTypes ||= path === '[Content_Types].xml';
    document ||= path === expected;
    p = next;
  }
  return p === end && contentTypes && document;
}

export function quoteEtag(etag: string): string {
  return etag.startsWith('"') ? etag : `"${etag}"`;
}

export function bareEtag(etag: string): string {
  return etag.length >= 2 && etag.startsWith('"') && etag.endsWith('"') ? etag.slice(1, -1) : etag;
}
