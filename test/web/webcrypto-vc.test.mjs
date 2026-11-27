import test from "node:test";
import assert from "node:assert/strict";
import {
  clearKey,
  computeJwkThumbprintShort,
  generateNonExtractableEs256Key,
  generateRandomNonceBase64Url,
  getKeyInfo,
  getOrCreateKeyInfo,
  signEs256,
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
