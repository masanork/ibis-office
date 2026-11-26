// SPDX-License-Identifier: AGPL-3.0-only
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  generateRecipientKeyPair,
  openSpotEnvelope,
  sealSpotEnvelope,
} from '../../www/spot/cose-envelope.js';
import { formatSpotCsv, formatSpotTsv } from '../../www/spot/export.js';
import { encodeQrMatrix, renderQrSvg } from '../../www/spot/qr.js';
import { createMemoryRelayStore, dispatchSpotRelay } from '../../worker/spot-relay.js';

test('COSE_Encrypt0 Spot envelope round-trips and rejects context or key tampering', async () => {
  const { publicJwk, privateJwk } = await generateRecipientKeyPair('demo-city-2026-01');
  const context = { tenantId: 'demo-city', formId: 'evac-checkin', version: '1.0.0' };
  const payload = { answers: { name: '山田 太郎', party_size: '3' }, submitted_at: '2026-11-26T00:00:00Z' };

  const sealed = await sealSpotEnvelope(publicJwk, context, payload);
  assert.match(sealed.envelope, /^spot1:[A-Za-z0-9_-]+$/);
  assert.match(sealed.envelopeId, /^[a-f0-9]{64}$/);

  const opened = await openSpotEnvelope(sealed.envelope, privateJwk, context);
  assert.equal(opened.envelopeId, sealed.envelopeId);
  assert.equal(opened.kid, 'demo-city-2026-01');
  assert.deepEqual(opened.payload, payload);

  await assert.rejects(
    openSpotEnvelope(sealed.envelope, privateJwk, { ...context, tenantId: 'other-city' }),
  );

  const otherKeys = await generateRecipientKeyPair('other');
  await assert.rejects(openSpotEnvelope(sealed.envelope, otherKeys.privateJwk, context));
});

test('Spot relay deduplicates envelopes idempotently and isolates tenants', async () => {
  const store = createMemoryRelayStore();
  const { publicJwk } = await generateRecipientKeyPair('demo-city-2026-01');
  const sealed = await sealSpotEnvelope(
    publicJwk,
    { tenantId: 'demo-city', formId: 'evac-checkin', version: '1.0.0' },
    { answers: { name: '佐藤 花子' } },
  );

  const submitBody = JSON.stringify({
    tenant_id: 'demo-city',
    items: [{ envelope_id: sealed.envelopeId, form_id: 'evac-checkin', envelope: sealed.envelope }],
  });

  const first = await dispatchSpotRelay(
    new Request('https://docs.sorane.dev/api/relay/v1/submit', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: submitBody,
    }),
    {},
    store,
  );
  assert.equal(first?.status, 200);
  assert.deepEqual(await first?.json(), {
    ok: true,
    tenant_id: 'demo-city',
    accepted: [sealed.envelopeId],
    duplicates: [],
  });

  // Resubmitting the exact same envelope_id (e.g. QR + online retry) is deduplicated.
  const second = await dispatchSpotRelay(
    new Request('https://docs.sorane.dev/api/relay/v1/submit', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: submitBody,
    }),
    {},
    store,
  );
  assert.equal(second?.status, 200);
  assert.deepEqual(await second?.json(), {
    ok: true,
    tenant_id: 'demo-city',
    accepted: [],
    duplicates: [sealed.envelopeId],
  });

  // Bearer token protection on inbox when configured.
  const unauthorized = await dispatchSpotRelay(
    new Request('https://docs.sorane.dev/api/relay/v1/inbox?tenant_id=demo-city'),
    { SPOT_RELAY_READ_TOKEN: 'secret-token' },
    store,
  );
  assert.equal(unauthorized?.status, 401);

  const inbox = await dispatchSpotRelay(
    new Request('https://docs.sorane.dev/api/relay/v1/inbox?tenant_id=demo-city', {
      headers: { authorization: 'Bearer secret-token' },
    }),
    { SPOT_RELAY_READ_TOKEN: 'secret-token' },
    store,
  );
  assert.equal(inbox?.status, 200);
  const inboxData = await inbox?.json();
  assert.equal(inboxData.items.length, 1);
  assert.equal(inboxData.items[0].envelope_id, sealed.envelopeId);

  const otherInbox = await dispatchSpotRelay(
    new Request('https://docs.sorane.dev/api/relay/v1/inbox?tenant_id=other-city'),
    {},
    store,
  );
  assert.equal(otherInbox?.status, 200);
  assert.deepEqual((await otherInbox?.json()).items, []);
});

test('Spot TSV and BOM CSV export formats decrypted records cleanly', () => {
  const records = [
    {
      envelope_id: 'a'.repeat(64),
      source: 'relay',
      payload: {
        submitted_at: '2026-11-26T10:00:00Z',
        answers: { name: '山田, 太郎', party_size: 3 },
      },
    },
  ];
  const tsv = formatSpotTsv(records);
  assert.equal(
    tsv,
    `${'a'.repeat(64)}\t2026-11-26T10:00:00Z\t{"submitted_at":"2026-11-26T10:00:00Z","answers":{"name":"山田, 太郎","party_size":3}}`,
  );
  const csv = formatSpotCsv(records);
  assert.equal(
    csv,
    `\uFEFFrecord_id,submitted_at,source,name,party_size\r\n${'a'.repeat(64)},2026-11-26T10:00:00Z,relay,"山田, 太郎",3\r\n`,
  );
});

test('QR encoder generates valid matrix finder patterns and SVG for Spot envelopes', async () => {
  const { publicJwk } = await generateRecipientKeyPair('demo-city-2026-01');
  const sealed = await sealSpotEnvelope(
    publicJwk,
    { tenantId: 'demo-city', formId: 'evac-checkin', version: '1.0.0' },
    { answers: { name: '山田 太郎', party_size: 2 } },
  );
  const matrix = encodeQrMatrix(sealed.envelope);
  assert.ok(matrix.version >= 1 && matrix.version <= 20);
  assert.equal(matrix.size, matrix.version * 4 + 17);
  // Top-left finder pattern center (3,3) and outer ring (0,0) must be dark.
  assert.equal(matrix.modules[0], 1);
  assert.equal(matrix.modules[3 * matrix.size + 3], 1);
  const svg = renderQrSvg(sealed.envelope);
  assert.ok(svg.startsWith('<svg ') && svg.includes('</svg>'));
});
