// SPDX-License-Identifier: AGPL-3.0-only
// Helpers for self-contained Form Manifest URLs (?m=...) and per-tenant read-token generation.

import { base64UrlDecode, base64UrlEncode, type SpotRecipientJwk } from './cose-envelope.js';

export interface SpotFormField {
  id: string;
  label: string;
  type: 'text' | 'number' | 'textarea' | 'date';
  required?: boolean;
  placeholder?: string;
  claim_path?: string;
}

export interface SpotFormManifest {
  schema: string;
  tenant_id: string;
  form_id: string;
  version: string;
  title: string;
  description?: string;
  relay_url: string;
  inbox_url?: string;
  recipient_key?: SpotRecipientJwk;
  fields: SpotFormField[];
}

export interface SpotReceiverBundle {
  schema: 'sorane.spot.receiver.v1';
  tenant_id: string;
  form_id: string;
  version: string;
  read_token?: string;
  private_jwk: SpotRecipientJwk;
}

export function encodeManifestParam(manifest: SpotFormManifest): string {
  const bytes = new TextEncoder().encode(JSON.stringify(manifest));
  return base64UrlEncode(bytes);
}

export function decodeManifestParam(encoded: string): SpotFormManifest {
  const bytes = base64UrlDecode(encoded.trim());
  const parsed = JSON.parse(new TextDecoder().decode(bytes)) as SpotFormManifest;
  if (!parsed || typeof parsed.tenant_id !== 'string' || typeof parsed.form_id !== 'string' || !Array.isArray(parsed.fields)) {
    throw new Error('Invalid encoded Spot manifest');
  }
  return parsed;
}

export function buildShareableFormUrl(origin: string, pathname: string, manifest: SpotFormManifest): string {
  const url = new URL(pathname, origin);
  url.search = '';
  url.searchParams.set('m', encodeManifestParam(manifest));
  return url.toString();
}

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

export async function encodeCompressedManifestParam(manifest: SpotFormManifest): Promise<string> {
  const raw = new TextEncoder().encode(JSON.stringify(manifest));
  if (typeof CompressionStream === 'undefined') {
    return base64UrlEncode(raw);
  }
  const stream = new Blob([toArrayBuffer(raw)]).stream().pipeThrough(new CompressionStream('deflate-raw'));
  const compressed = new Uint8Array(await new Response(stream).arrayBuffer());
  return base64UrlEncode(compressed);
}

export async function decodeCompressedManifestParam(encoded: string): Promise<SpotFormManifest> {
  const compressed = base64UrlDecode(encoded.trim());
  if (typeof DecompressionStream === 'undefined') {
    throw new Error('DecompressionStream unavailable for ?mz= manifest');
  }
  const stream = new Blob([toArrayBuffer(compressed)]).stream().pipeThrough(new DecompressionStream('deflate-raw'));
  const raw = new Uint8Array(await new Response(stream).arrayBuffer());
  const parsed = JSON.parse(new TextDecoder().decode(raw)) as SpotFormManifest;
  if (!parsed || typeof parsed.tenant_id !== 'string' || typeof parsed.form_id !== 'string' || !Array.isArray(parsed.fields)) {
    throw new Error('Invalid compressed Spot manifest');
  }
  return parsed;
}

export async function buildCompactShareableFormUrl(
  origin: string,
  pathname: string,
  manifest: SpotFormManifest,
): Promise<string> {
  const url = new URL(pathname, origin);
  url.search = '';
  if (typeof CompressionStream !== 'undefined') {
    url.searchParams.set('mz', await encodeCompressedManifestParam(manifest));
  } else {
    url.searchParams.set('m', encodeManifestParam(manifest));
  }
  return url.toString();
}

export async function generateReadToken(): Promise<{ readToken: string; readTokenHash: string }> {
  const random = crypto.getRandomValues(new Uint8Array(32));
  const readToken = base64UrlEncode(random);
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(readToken)));
  const readTokenHash = Array.from(digest, (b) => b.toString(16).padStart(2, '0')).join('');
  return { readToken, readTokenHash };
}
