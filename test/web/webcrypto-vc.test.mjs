import test from "node:test";
import assert from "node:assert/strict";
import {
  clearKey,
  computeJwkThumbprintShort,
  createCrossDeviceVcSession,
  decryptCrossDevicePayload,
  encryptCrossDevicePayload,
  generateNonExtractableEs256Key,
  generateRandomNonceBase64Url,
  getKeyInfo,
  getOrCreateKeyInfo,
  isDigitalCredentialsSupported,
  signEs256,
  submitOid4vpDirectPost,
  verifyEs256,
} from "../../www/sorane-webcrypto-vc.js";

test("WebCrypto ES256 keypair is non-extractable and yields deterministic RFC 7638 thumbprint", async () => {
  await clearKey("holder-test");
  const created = await generateNonExtractableEs256Key("holder-test");
  assert.equal(created.role, "holder-test");
  assert.equal(created.extractable, false);
  assert.equal(created.backend, "webcrypto_indexeddb");
  assert.equal(created.publicJwk.kty, "EC");
  assert.equal(created.publicJwk.crv, "P-256");
  assert.equal(created.publicJwk.alg, "ES256");
  assert.equal(typeof created.publicJwk.x, "string");
  assert.equal(typeof created.publicJwk.y, "string");
  assert.equal(created.thumbprint.length, 12);

  // Recomputing RFC 7638 thumbprint from public JWK must match identically.
  const recomputed = await computeJwkThumbprintShort(created.publicJwk);
  assert.equal(recomputed, created.thumbprint);

  // Subsequent getOrCreateKeyInfo returns the persisted key instead of regenerating.
  const loaded = await getOrCreateKeyInfo("holder-test");
  assert.equal(loaded.thumbprint, created.thumbprint);
  assert.deepEqual(loaded.publicJwk, created.publicJwk);

  await clearKey("holder-test");
  const afterClear = await getKeyInfo("holder-test");
  assert.equal(afterClear, null);
});

test("signEs256 produces a valid 64-byte IEEE P1363 signature verifiable by public JWK", async () => {
  await clearKey("holder-sign-test");
  const info = await getOrCreateKeyInfo("holder-sign-test");
  const signingInput = "eyJhbGciOiJFUzI1NiIsInR5cCI6ImtiK2p3dCJ9.eyJhdWQiOiJodHRwczovL3ZlcmlmaWVyLmV4YW1wbGUiLCJub25jZSI6Im4tMTIzIiwiaWF0IjoxNzM1MDAwMDAwLCJzZF9oYXNoIjoiYWJjIn0";

  const sig = await signEs256("holder-sign-test", signingInput);
  assert.ok(sig instanceof Uint8Array);
  assert.equal(sig.length, 64);

  const ok = await verifyEs256(info.publicJwk, signingInput, sig);
  assert.equal(ok, true);

  const tamperedOk = await verifyEs256(info.publicJwk, signingInput + "x", sig);
  assert.equal(tamperedOk, false);

  await clearKey("holder-sign-test");
});

test("generateRandomNonceBase64Url generates distinct prefixed nonces", () => {
  const a = generateRandomNonceBase64Url();
  const b = generateRandomNonceBase64Url();
  assert.ok(a.startsWith("n-"));
  assert.ok(b.startsWith("n-"));
  assert.notEqual(a, b);
});

test("cross-device ECDH P-256 + AES-256-GCM envelope encrypts and decrypts VC payload", async () => {
  const session = await createCrossDeviceVcSession();
  assert.ok(session.sessionId.startsWith("n-"));
  assert.equal(session.publicJwk.kty, "EC");
  assert.equal(session.publicJwk.crv, "P-256");

  const sdJwtSample = "eyJhbGciOiJFUzI1NiJ9.eyJfc2QiOlsiYWJjIl19.sig~WyJzYWx0IiwibmFtZSIsIlx1NTMzYSJd~";
  const envelope = await encryptCrossDevicePayload(session.publicJwk, sdJwtSample);
  assert.equal(envelope.alg, "ECDH-ES+A256GCM");

  const decrypted = await decryptCrossDevicePayload(session.privateKey, envelope);
  assert.equal(decrypted, sdJwtSample);

  // Tampering with ciphertext must fail AES-GCM authentication tag
  const tampered = { ...envelope, ct: envelope.ct.slice(0, -2) + "AA" };
  await assert.rejects(() => decryptCrossDevicePayload(session.privateKey, tampered));
});

test("submitOid4vpDirectPost falls back to /api/vc/direct-post relay when direct fetch fails and allowRelay=true", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    if (url === "https://verifier.example.jp/oid4vp/post") {
      throw new TypeError("Failed to fetch (CORS)");
    }
    if (url === "/api/vc/direct-post") {
      return new Response(
        JSON.stringify({
          status: 200,
          response_uri: "https://verifier.example.jp/oid4vp/post",
          body: '{"ok":true}',
          redirect_uri: "https://verifier.example.jp/done",
          relayed: true,
        }),
        { status: 200, headers: { "content-type": "application/json" } }
      );
    }
    throw new Error(`Unexpected URL: ${url}`);
  };

  try {
    await assert.rejects(() =>
      submitOid4vpDirectPost("https://verifier.example.jp/oid4vp/post", "vp_token=abc", {
        allowRelay: false,
      })
    );

    const relayed = await submitOid4vpDirectPost(
      "https://verifier.example.jp/oid4vp/post",
      "vp_token=abc",
      { allowRelay: true }
    );
    assert.equal(relayed.relayed, true);
    assert.equal(relayed.status, 200);
    assert.equal(relayed.redirect_uri, "https://verifier.example.jp/done");
    assert.equal(calls.length, 3);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("isDigitalCredentialsSupported returns false in Node without DigitalCredential", () => {
  assert.equal(isDigitalCredentialsSupported(), false);
});
