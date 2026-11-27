/**
 * Sorane WebCrypto VC Keystore (Phase 4B)
 *
 * Manages non-extractable ES256 (ECDSA P-256) keypairs in browser IndexedDB
 * (`sorane-vc-keys`) via `SubtleCrypto` (`extractable: false`).
 *
 * Private keys never leave WebCrypto and are never serialized to GitHub Vault or Git.
 * Only public JWK + RFC 7638 thumbprint + 64-byte IEEE P1363 signatures are exposed.
 */

const DB_NAME = "sorane-vc-keys";
const DB_VERSION = 1;
const STORE_NAME = "keypairs";

const memoryStore = new Map();

function getCrypto() {
  const cryptoObj = globalThis.crypto;
  if (!cryptoObj || !cryptoObj.subtle) {
    throw new Error("WebCrypto SubtleCrypto is not available in this environment.");
  }
  return cryptoObj;
}

function bytesToBase64Url(bytes) {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  const b64 =
    typeof btoa === "function"
      ? btoa(binary)
      : Buffer.from(bytes).toString("base64");
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function base64UrlToBytes(b64url) {
  const padded = b64url.replace(/-/g, "+").replace(/_/g, "/") +
    "=".repeat((4 - (b64url.length % 4)) % 4);
  if (typeof atob === "function") {
    const bin = atob(padded);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i += 1) {
      out[i] = bin.charCodeAt(i);
    }
    return out;
  }
  return new Uint8Array(Buffer.from(padded, "base64"));
}

/**
 * RFC 7638 JWK thumbprint for EC P-256 public JWK, truncated to 12 chars
 * to match `Es256KeyPair::thumbprint_short` in `ibis-doc-model`.
 */
export async function computeJwkThumbprintShort(publicJwk) {
  const cryptoObj = getCrypto();
  if (!publicJwk || publicJwk.kty !== "EC" || publicJwk.crv !== "P-256") {
    throw new Error("Expected an EC P-256 public JWK.");
  }
  const canonical = JSON.stringify({
    crv: "P-256",
    kty: "EC",
    x: publicJwk.x,
    y: publicJwk.y,
  });
  const digest = await cryptoObj.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(canonical)
  );
  return bytesToBase64Url(new Uint8Array(digest)).slice(0, 12);
}

function openKeyDb() {
  if (typeof indexedDB === "undefined") {
    return Promise.resolve(null);
  }
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME, { keyPath: "role" });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error || new Error("IndexedDB open failed"));
  });
}

async function idbGet(role) {
  const db = await openKeyDb();
  if (!db) {
    return memoryStore.get(role) || null;
  }
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, "readonly");
    const store = tx.objectStore(STORE_NAME);
    const req = store.get(role);
    req.onsuccess = () => {
      db.close();
      resolve(req.result || null);
    };
    req.onerror = () => {
      db.close();
      reject(req.error || new Error("IndexedDB get failed"));
    };
  });
}

async function idbPut(record) {
  const db = await openKeyDb();
  if (!db) {
    memoryStore.set(record.role, record);
    return;
  }
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    const req = store.put(record);
    req.onsuccess = () => {
      db.close();
      resolve();
    };
    req.onerror = () => {
      db.close();
      reject(req.error || new Error("IndexedDB put failed"));
    };
  });
}

async function idbDelete(role) {
  const db = await openKeyDb();
  if (!db) {
    memoryStore.delete(role);
    return;
  }
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    const req = store.delete(role);
    req.onsuccess = () => {
      db.close();
      resolve();
    };
    req.onerror = () => {
      db.close();
      reject(req.error || new Error("IndexedDB delete failed"));
    };
  });
}

export async function generateNonExtractableEs256Key(role = "holder") {
  const cryptoObj = getCrypto();
  const keyPair = await cryptoObj.subtle.generateKey(
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["sign", "verify"]
  );
  const publicJwk = await cryptoObj.subtle.exportKey("jwk", keyPair.publicKey);
  const cleanPublicJwk = {
    kty: "EC",
    crv: "P-256",
    x: publicJwk.x,
    y: publicJwk.y,
    alg: "ES256",
  };
  const thumbprint = await computeJwkThumbprintShort(cleanPublicJwk);
  const record = {
    role,
    privateKey: keyPair.privateKey,
    publicKey: keyPair.publicKey,
    publicJwk: cleanPublicJwk,
    thumbprint,
    createdAt: new Date().toISOString(),
  };
  await idbPut(record);
  return {
    role,
    publicJwk: cleanPublicJwk,
    thumbprint,
    extractable: false,
    backend: "webcrypto_indexeddb",
  };
}

export async function getOrCreateKeyInfo(role = "holder") {
  const existing = await idbGet(role);
  if (existing && existing.privateKey && existing.publicJwk) {
    return {
      role,
      publicJwk: existing.publicJwk,
      thumbprint: existing.thumbprint,
      extractable: false,
      backend: "webcrypto_indexeddb",
    };
  }
  return generateNonExtractableEs256Key(role);
}

export async function getKeyInfo(role = "holder") {
  const existing = await idbGet(role);
  if (!existing || !existing.publicJwk) {
    return null;
  }
  return {
    role,
    publicJwk: existing.publicJwk,
    thumbprint: existing.thumbprint,
    extractable: false,
    backend: "webcrypto_indexeddb",
  };
}

export async function clearKey(role = "holder") {
  await idbDelete(role);
}

/**
 * Signs `signingInput` (UTF-8 string or Uint8Array) with the stored non-extractable
 * P-256 private key and returns raw 64-byte IEEE P1363 `r || s` signature.
 */
export async function signEs256(role, signingInput) {
  const cryptoObj = getCrypto();
  let existing = await idbGet(role);
  if (!existing || !existing.privateKey) {
    await generateNonExtractableEs256Key(role);
    existing = await idbGet(role);
  }
  const data =
    typeof signingInput === "string"
      ? new TextEncoder().encode(signingInput)
      : signingInput;
  const sigBuf = await cryptoObj.subtle.sign(
    { name: "ECDSA", hash: { name: "SHA-256" } },
    existing.privateKey,
    data
  );
  const sigBytes = new Uint8Array(sigBuf);
  if (sigBytes.length !== 64) {
    throw new Error(`Expected 64-byte IEEE P1363 signature, got ${sigBytes.length}`);
  }
  return sigBytes;
}

export async function verifyEs256(publicJwk, signingInput, sigBytes) {
  const cryptoObj = getCrypto();
  const key = await cryptoObj.subtle.importKey(
    "jwk",
    { kty: "EC", crv: "P-256", x: publicJwk.x, y: publicJwk.y },
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["verify"]
  );
  const data =
    typeof signingInput === "string"
      ? new TextEncoder().encode(signingInput)
      : signingInput;
  return cryptoObj.subtle.verify(
    { name: "ECDSA", hash: { name: "SHA-256" } },
    key,
    sigBytes,
    data
  );
}

export function generateRandomNonceBase64Url(byteLength = 12) {
  const cryptoObj = getCrypto();
  const bytes = new Uint8Array(byteLength);
  cryptoObj.getRandomValues(bytes);
  return `n-${bytesToBase64Url(bytes)}`;
}

// Install browser global bridges for WASM (`ibis-office-shell::vc_transport::wasm`).
if (typeof window !== "undefined") {
  window.soraneVcKeystore = {
    computeJwkThumbprintShort,
    generateNonExtractableEs256Key,
    getOrCreateKeyInfo,
    getKeyInfo,
    clearKey,
    signEs256,
    verifyEs256,
    generateRandomNonceBase64Url,
    bytesToBase64Url,
    base64UrlToBytes,
  };

  window.ibisOfficeWebCryptoHolderInfo = async () => {
    const info = await getOrCreateKeyInfo("holder");
    return JSON.stringify(info);
  };

  window.ibisOfficeWebCryptoSignHolder = async (signingInput) => {
    const sig = await signEs256("holder", signingInput);
    return bytesToBase64Url(sig);
  };
}
