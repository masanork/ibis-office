// SPDX-License-Identifier: AGPL-3.0-only
// Minimal RFC 9052 COSE_Encrypt0 envelope using Web Crypto (ECDH-ES + HKDF-SHA256 + AES-256-GCM).

export interface SpotRecipientJwk extends JsonWebKey {
  kty: 'EC';
  crv: 'P-256';
  kid?: string;
  x: string;
  y: string;
  d?: string;
}

export interface SpotEnvelopeContext {
  tenantId: string;
  formId: string;
  version: string;
}

export interface SealedSpotEnvelope {
  envelope: string;
  envelopeId: string;
  bytes: Uint8Array;
}

export interface OpenedSpotEnvelope<T = unknown> {
  envelopeId: string;
  kid: string | null;
  payload: T;
}

type CborValue =
  | number
  | string
  | Uint8Array
  | CborValue[]
  | Map<number | string, CborValue>;

const ENVELOPE_PREFIX = 'spot1:';
const COSE_ENCRYPT0_TAG = 16;
const ALG_ECDH_ES_HKDF_256 = -29;

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

export function base64UrlEncode(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
}

export function base64UrlDecode(text: string): Uint8Array {
  const padded = text.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((text.length + 3) % 4);
  const binary = atob(padded);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    out[i] = binary.charCodeAt(i);
  }
  return out;
}

function concatBytes(...chunks: Uint8Array[]): Uint8Array {
  const total = chunks.reduce((sum, c) => sum + c.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

function encodeHeader(major: number, len: number): Uint8Array {
  const mt = major << 5;
  if (len < 24) return new Uint8Array([mt | len]);
  if (len <= 0xff) return new Uint8Array([mt | 24, len]);
  if (len <= 0xffff) return new Uint8Array([mt | 25, len >> 8, len & 0xff]);
  return new Uint8Array([
    mt | 26,
    (len >>> 24) & 0xff,
    (len >>> 16) & 0xff,
    (len >>> 8) & 0xff,
    len & 0xff,
  ]);
}

export function encodeCbor(value: CborValue): Uint8Array {
  if (typeof value === 'number') {
    if (!Number.isInteger(value)) throw new Error('CBOR floats are not used in Spot envelopes');
    if (value >= 0) return encodeHeader(0, value);
    return encodeHeader(1, -1 - value);
  }
  if (typeof value === 'string') {
    const utf8 = new TextEncoder().encode(value);
    return concatBytes(encodeHeader(3, utf8.length), utf8);
  }
  if (value instanceof Uint8Array) {
    return concatBytes(encodeHeader(2, value.length), value);
  }
  if (Array.isArray(value)) {
    const items = value.map((item) => encodeCbor(item));
    return concatBytes(encodeHeader(4, value.length), ...items);
  }
  if (value instanceof Map) {
    const parts: Uint8Array[] = [encodeHeader(5, value.size)];
    for (const [k, v] of value.entries()) {
      parts.push(encodeCbor(k), encodeCbor(v));
    }
    return concatBytes(...parts);
  }
  throw new Error('Unsupported CBOR value');
}

function decodeCborAt(buf: Uint8Array, offset: number): { value: unknown; next: number; tag?: number } {
  if (offset >= buf.length) throw new Error('Unexpected end of CBOR');
  const first = buf[offset];
  const major = first >> 5;
  const ai = first & 0x1f;
  let pos = offset + 1;
  let arg = ai;
  if (ai === 24) {
    arg = buf[pos++];
  } else if (ai === 25) {
    arg = (buf[pos] << 8) | buf[pos + 1];
    pos += 2;
  } else if (ai === 26) {
    arg = ((buf[pos] << 24) | (buf[pos + 1] << 16) | (buf[pos + 2] << 8) | buf[pos + 3]) >>> 0;
    pos += 4;
  } else if (ai >= 27) {
    throw new Error(`Unsupported CBOR additional info: ${ai}`);
  }

  switch (major) {
    case 0:
      return { value: arg, next: pos };
    case 1:
      return { value: -1 - arg, next: pos };
    case 2: {
      const end = pos + arg;
      if (end > buf.length) throw new Error('CBOR byte string out of bounds');
      return { value: buf.slice(pos, end), next: end };
    }
    case 3: {
      const end = pos + arg;
      if (end > buf.length) throw new Error('CBOR text string out of bounds');
      return { value: new TextDecoder().decode(buf.slice(pos, end)), next: end };
    }
    case 4: {
      const arr: unknown[] = [];
      for (let i = 0; i < arg; i++) {
        const item = decodeCborAt(buf, pos);
        arr.push(item.value);
        pos = item.next;
      }
      return { value: arr, next: pos };
    }
    case 5: {
      const map = new Map<number | string, unknown>();
      for (let i = 0; i < arg; i++) {
        const k = decodeCborAt(buf, pos);
        const v = decodeCborAt(buf, k.next);
        map.set(k.value as number | string, v.value);
        pos = v.next;
      }
      return { value: map, next: pos };
    }
    case 6: {
      const inner = decodeCborAt(buf, pos);
      return { value: inner.value, next: inner.next, tag: arg };
    }
    default:
      throw new Error(`Unsupported CBOR major type: ${major}`);
  }
}

async function compressPayload(raw: Uint8Array): Promise<Uint8Array> {
  if (typeof CompressionStream === 'undefined') {
    return concatBytes(new Uint8Array([0x00]), raw);
  }
  const stream = new Blob([toArrayBuffer(raw)]).stream().pipeThrough(new CompressionStream('deflate-raw'));
  const compressed = new Uint8Array(await new Response(stream).arrayBuffer());
  if (compressed.length < raw.length) {
    return concatBytes(new Uint8Array([0x01]), compressed);
  }
  return concatBytes(new Uint8Array([0x00]), raw);
}

async function decompressPayload(encoded: Uint8Array): Promise<Uint8Array> {
  if (encoded.length === 0) throw new Error('Empty payload');
  const flag = encoded[0];
  const body = encoded.slice(1);
  if (flag === 0x00) return body;
  if (flag !== 0x01) throw new Error(`Unknown compression flag: ${flag}`);
  if (typeof DecompressionStream === 'undefined') {
    throw new Error('DecompressionStream unavailable for deflate-raw payload');
  }
  const stream = new Blob([toArrayBuffer(body)]).stream().pipeThrough(new DecompressionStream('deflate-raw'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', toArrayBuffer(bytes)));
  return Array.from(digest, (b) => b.toString(16).padStart(2, '0')).join('');
}

function buildExternalAad(ctx: SpotEnvelopeContext): Uint8Array {
  return encodeCbor(['sorane.spot.v1', ctx.tenantId, ctx.formId, ctx.version]);
}

function buildEncStructure(protectedBytes: Uint8Array, externalAad: Uint8Array): Uint8Array {
  return encodeCbor(['Encrypt0', protectedBytes, externalAad]);
}

async function deriveAesKey(
  privateKey: CryptoKey,
  publicKey: CryptoKey,
  protectedBytes: Uint8Array,
): Promise<CryptoKey> {
  const sharedBits = await crypto.subtle.deriveBits(
    { name: 'ECDH', public: publicKey },
    privateKey,
    256,
  );
  const hkdfKey = await crypto.subtle.importKey('raw', sharedBits, 'HKDF', false, ['deriveKey']);
  const info = encodeCbor(['sorane.spot.hkdf.v1', protectedBytes]);
  return crypto.subtle.deriveKey(
    { name: 'HKDF', hash: 'SHA-256', salt: new Uint8Array(32), info: toArrayBuffer(info) },
    hkdfKey,
    { name: 'AES-GCM', length: 256 },
    false,
    ['encrypt', 'decrypt'],
  );
}

function coseKeyFromP256Jwk(jwk: JsonWebKey): Map<number | string, CborValue> {
  if (!jwk.x || !jwk.y) throw new Error('Invalid P-256 JWK');
  return new Map<number | string, CborValue>([
    [1, 2], // kty: EC2
    [-1, 1], // crv: P-256
    [-2, base64UrlDecode(jwk.x)],
    [-3, base64UrlDecode(jwk.y)],
  ]);
}

function p256JwkFromCoseKey(coseKey: Map<number | string, unknown>): JsonWebKey {
  const x = coseKey.get(-2);
  const y = coseKey.get(-3);
  if (!(x instanceof Uint8Array) || !(y instanceof Uint8Array)) {
    throw new Error('Invalid COSE EC2 key');
  }
  return { kty: 'EC', crv: 'P-256', x: base64UrlEncode(x), y: base64UrlEncode(y) };
}

export async function generateRecipientKeyPair(kid: string): Promise<{
  publicJwk: SpotRecipientJwk;
  privateJwk: SpotRecipientJwk;
}> {
  const pair = await crypto.subtle.generateKey(
    { name: 'ECDH', namedCurve: 'P-256' },
    true,
    ['deriveBits'],
  );
  const publicJwk = (await crypto.subtle.exportKey('jwk', pair.publicKey)) as SpotRecipientJwk;
  const privateJwk = (await crypto.subtle.exportKey('jwk', pair.privateKey)) as SpotRecipientJwk;
  publicJwk.kid = kid;
  privateJwk.kid = kid;
  return { publicJwk, privateJwk };
}

export async function sealSpotEnvelope(
  recipientPublicJwk: SpotRecipientJwk,
  context: SpotEnvelopeContext,
  payload: unknown,
): Promise<SealedSpotEnvelope> {
  const recipientKey = await crypto.subtle.importKey(
    'jwk',
    recipientPublicJwk,
    { name: 'ECDH', namedCurve: 'P-256' },
    false,
    [],
  );
  const ephemeral = await crypto.subtle.generateKey(
    { name: 'ECDH', namedCurve: 'P-256' },
    true,
    ['deriveBits'],
  );
  const ephemeralJwk = await crypto.subtle.exportKey('jwk', ephemeral.publicKey);

  const protectedMap = new Map<number | string, CborValue>([[1, ALG_ECDH_ES_HKDF_256]]);
  if (recipientPublicJwk.kid) {
    protectedMap.set(4, new TextEncoder().encode(recipientPublicJwk.kid));
  }
  const protectedBytes = encodeCbor(protectedMap);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const aesKey = await deriveAesKey(ephemeral.privateKey, recipientKey, protectedBytes);

  const rawJson = new TextEncoder().encode(JSON.stringify(payload));
  const plaintext = await compressPayload(rawJson);
  const aad = buildEncStructure(protectedBytes, buildExternalAad(context));
  const ciphertext = new Uint8Array(
    await crypto.subtle.encrypt(
      { name: 'AES-GCM', iv: toArrayBuffer(iv), additionalData: toArrayBuffer(aad) },
      aesKey,
      toArrayBuffer(plaintext),
    ),
  );

  const unprotectedMap = new Map<number | string, CborValue>([
    [-1, coseKeyFromP256Jwk(ephemeralJwk)],
    [5, iv],
  ]);
  const bodyBytes = encodeCbor([protectedBytes, unprotectedMap, ciphertext]);
  const coseBytes = concatBytes(encodeHeader(6, COSE_ENCRYPT0_TAG), bodyBytes);
  const envelopeId = await sha256Hex(coseBytes);
  return {
    envelope: `${ENVELOPE_PREFIX}${base64UrlEncode(coseBytes)}`,
    envelopeId,
    bytes: coseBytes,
  };
}

export async function openSpotEnvelope<T = unknown>(
  envelopeStr: string,
  recipientPrivateJwk: SpotRecipientJwk,
  context: SpotEnvelopeContext,
): Promise<OpenedSpotEnvelope<T>> {
  if (!envelopeStr.startsWith(ENVELOPE_PREFIX)) {
    throw new Error('Invalid Spot envelope prefix');
  }
  const coseBytes = base64UrlDecode(envelopeStr.slice(ENVELOPE_PREFIX.length));
  const envelopeId = await sha256Hex(coseBytes);
  const decoded = decodeCborAt(coseBytes, 0);
  if (decoded.tag !== COSE_ENCRYPT0_TAG || !Array.isArray(decoded.value) || decoded.value.length !== 3) {
    throw new Error('Invalid COSE_Encrypt0 structure');
  }

  const [protectedBytes, unprotectedMap, ciphertext] = decoded.value;
  if (!(protectedBytes instanceof Uint8Array) || !(unprotectedMap instanceof Map) || !(ciphertext instanceof Uint8Array)) {
    throw new Error('Malformed COSE_Encrypt0 fields');
  }

  const protectedDecoded = decodeCborAt(protectedBytes, 0).value;
  if (!(protectedDecoded instanceof Map) || protectedDecoded.get(1) !== ALG_ECDH_ES_HKDF_256) {
    throw new Error('Unsupported COSE algorithm');
  }
  const rawKid = protectedDecoded.get(4);
  const kid = rawKid instanceof Uint8Array ? new TextDecoder().decode(rawKid) : null;

  const epkCose = unprotectedMap.get(-1);
  const iv = unprotectedMap.get(5);
  if (!(epkCose instanceof Map) || !(iv instanceof Uint8Array) || iv.length !== 12) {
    throw new Error('Missing ephemeral key or IV');
  }

  const privateKey = await crypto.subtle.importKey(
    'jwk',
    recipientPrivateJwk,
    { name: 'ECDH', namedCurve: 'P-256' },
    false,
    ['deriveBits'],
  );
  const ephemeralPublicKey = await crypto.subtle.importKey(
    'jwk',
    p256JwkFromCoseKey(epkCose),
    { name: 'ECDH', namedCurve: 'P-256' },
    false,
    [],
  );

  const aesKey = await deriveAesKey(privateKey, ephemeralPublicKey, protectedBytes);
  const aad = buildEncStructure(protectedBytes, buildExternalAad(context));
  const decrypted = new Uint8Array(
    await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv: toArrayBuffer(iv), additionalData: toArrayBuffer(aad) },
      aesKey,
      toArrayBuffer(ciphertext),
    ),
  );

  const rawJson = await decompressPayload(decrypted);
  return {
    envelopeId,
    kid,
    payload: JSON.parse(new TextDecoder().decode(rawJson)) as T,
  };
}
