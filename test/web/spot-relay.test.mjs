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
import {
  buildCompactShareableFormUrl,
  buildShareableFormUrl,
  decodeCompressedManifestParam,
  decodeManifestParam,
  generateReadToken,
} from '../../www/spot/tenant-setup.js';
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
        verification: { method: 'self_asserted', trust_status: 'unverified' },
        answers: { name: '山田, 太郎', input_helper: '代理 花子', group_id: 'grp-1' },
      },
    },
  ];
  const tsv = formatSpotTsv(records);
  assert.equal(
    tsv,
    `${'a'.repeat(64)}\t2026-11-26T10:00:00Z\t{"submitted_at":"2026-11-26T10:00:00Z","verification":{"method":"self_asserted","trust_status":"unverified"},"answers":{"name":"山田, 太郎","input_helper":"代理 花子","group_id":"grp-1"}}`,
  );
  const csv = formatSpotCsv(records);
  assert.equal(
    csv,
    `\uFEFFrecord_id,submitted_at,source,trust_status,name,input_helper,group_id\r\n${'a'.repeat(64)},2026-11-26T10:00:00Z,relay,unverified,"山田, 太郎",代理 花子,grp-1\r\n`,
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

test('Tenant registration enforces per-tenant Bearer read_token and ?m= manifest round-trips', async () => {
  const store = createMemoryRelayStore();
  const { readToken, readTokenHash } = await generateReadToken();

  const reg = await dispatchSpotRelay(
    new Request('https://spot.sorane.dev/api/relay/v1/tenants', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ tenant_id: 'chiyoda-2026', read_token_hash: readTokenHash }),
    }),
    {},
    store,
  );
  assert.equal(reg?.status, 201);

  const dup = await dispatchSpotRelay(
    new Request('https://spot.sorane.dev/api/relay/v1/tenants', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ tenant_id: 'chiyoda-2026', read_token_hash: readTokenHash }),
    }),
    {},
    store,
  );
  assert.equal(dup?.status, 409);

  const denied = await dispatchSpotRelay(
    new Request('https://spot.sorane.dev/api/relay/v1/inbox?tenant_id=chiyoda-2026'),
    {},
    store,
  );
  assert.equal(denied?.status, 401);

  const wrong = await dispatchSpotRelay(
    new Request('https://spot.sorane.dev/api/relay/v1/inbox?tenant_id=chiyoda-2026', {
      headers: { authorization: 'Bearer wrong-token' },
    }),
    {},
    store,
  );
  assert.equal(wrong?.status, 401);

  const ok = await dispatchSpotRelay(
    new Request('https://spot.sorane.dev/api/relay/v1/inbox?tenant_id=chiyoda-2026', {
      headers: { authorization: `Bearer ${readToken}` },
    }),
    {},
    store,
  );
  assert.equal(ok?.status, 200);

  const shareUrl = buildShareableFormUrl('https://spot.sorane.dev', '/', {
    schema: 'sorane.spot.form.v1',
    tenant_id: 'chiyoda-2026',
    form_id: 'evac-checkin',
    version: '1.1.0',
    title: '千代田区避難所受付',
    relay_url: 'https://spot.sorane.dev/api/relay/v1/submit',
    fields: [{ id: 'name', label: '氏名', type: 'text', required: true, claim_path: 'person.full_name' }],
  });
  const encoded = new URL(shareUrl).searchParams.get('m');
  assert.ok(encoded);
  const decoded = decodeManifestParam(encoded);
  assert.equal(decoded.tenant_id, 'chiyoda-2026');
  assert.equal(decoded.fields[0].claim_path, 'person.full_name');
});

test('QR v40-L supports large ?mz= compressed shareable manifest URLs', async () => {
  const { publicJwk } = await generateRecipientKeyPair('chiyoda-2026-01');
  const compactUrl = await buildCompactShareableFormUrl('https://spot.sorane.dev', '/', {
    schema: 'sorane.spot.form.v1',
    tenant_id: 'chiyoda-2026',
    form_id: 'evac-checkin',
    version: '1.1.0',
    title: '千代田区避難所受付票（個人単位）',
    relay_url: 'https://spot.sorane.dev/api/relay/v1/submit',
    inbox_url: 'https://spot.sorane.dev/api/relay/v1/inbox',
    recipient_key: publicJwk,
    fields: [
      { id: 'name', label: '氏名', type: 'text', required: true, claim_path: 'person.full_name' },
      { id: 'birth_date', label: '生年月日', type: 'text', claim_path: 'person.birth_date' },
      { id: 'address', label: '住所', type: 'text', claim_path: 'person.address' },
      { id: 'shelter', label: '避難場所', type: 'text' },
      { id: 'needs', label: '要配慮事項', type: 'textarea' },
      { id: 'input_helper', label: '入力補助者', type: 'text' },
      { id: 'group_id', label: '同行者グループID', type: 'text' },
    ],
  });
  const mz = new URL(compactUrl).searchParams.get('mz');
  assert.ok(mz);
  const decoded = await decodeCompressedManifestParam(mz);
  assert.equal(decoded.tenant_id, 'chiyoda-2026');
  assert.equal(decoded.fields.length, 7);

  // Also verify QR matrix & SVG for > 1,000 byte payloads (Version > 20).
  const largePayload = 'spot1:' + 'A'.repeat(1200);
  const matrix = encodeQrMatrix(largePayload);
  assert.ok(matrix.version > 20 && matrix.version <= 40);
  const svg = renderQrSvg(compactUrl);
  assert.ok(svg.startsWith('<svg ') && svg.includes('</svg>'));
});
