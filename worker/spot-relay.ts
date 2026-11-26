// SPDX-License-Identifier: AGPL-3.0-only
// Generic, schema-less, multi-tenant zero-knowledge relay for Spot encrypted form envelopes.

export interface SpotRelayItemInput {
  envelope_id: string;
  form_id: string;
  envelope: string;
}

export interface SpotRelayStoredItem extends SpotRelayItemInput {
  tenant_id: string;
  received_at: number;
}

export interface SpotRelayStore {
  putBatch(tenantId: string, items: SpotRelayItemInput[], now: number): Promise<{
    accepted: string[];
    duplicates: string[];
  }>;
  list(tenantId: string, since: number): Promise<SpotRelayStoredItem[]>;
}

export interface SpotRelayEnv {
  SPOT_RELAY_READ_TOKEN?: string;
}

const TENANT_OR_FORM_RE = /^[a-zA-Z0-9._-]{1,64}$/;
const ENVELOPE_ID_RE = /^[a-f0-9]{64}$/;
const MAX_BATCH_ITEMS = 50;
const MAX_ENVELOPE_CHARS = 16_384;

const CORS_HEADERS: Record<string, string> = {
  'access-control-allow-origin': '*',
  'access-control-allow-methods': 'GET, POST, OPTIONS',
  'access-control-allow-headers': 'content-type, authorization',
  'cache-control': 'no-store',
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      'content-type': 'application/json; charset=utf-8',
      ...CORS_HEADERS,
    },
  });
}

export function createMemoryRelayStore(): SpotRelayStore {
  const byTenant = new Map<string, Map<string, SpotRelayStoredItem>>();
  return {
    async putBatch(tenantId, items, now) {
      let bucket = byTenant.get(tenantId);
      if (!bucket) {
        bucket = new Map();
        byTenant.set(tenantId, bucket);
      }
      const accepted: string[] = [];
      const duplicates: string[] = [];
      for (const item of items) {
        if (bucket.has(item.envelope_id)) {
          duplicates.push(item.envelope_id);
          continue;
        }
        bucket.set(item.envelope_id, {
          ...item,
          tenant_id: tenantId,
          received_at: now,
        });
        accepted.push(item.envelope_id);
      }
      return { accepted, duplicates };
    },
    async list(tenantId, since) {
      const bucket = byTenant.get(tenantId);
      if (!bucket) return [];
      return Array.from(bucket.values())
        .filter((item) => item.received_at >= since)
        .sort((a, b) => a.received_at - b.received_at);
    },
  };
}

const defaultStore = createMemoryRelayStore();

export async function dispatchSpotRelay(
  request: Request,
  env: SpotRelayEnv = {},
  store: SpotRelayStore = defaultStore,
): Promise<Response | null> {
  const url = new URL(request.url);
  if (!url.pathname.startsWith('/api/relay/v1/')) {
    return null;
  }

  if (request.method === 'OPTIONS') {
    return new Response(null, { status: 204, headers: CORS_HEADERS });
  }

  if (url.pathname === '/api/relay/v1/submit' && request.method === 'POST') {
    let body: { tenant_id?: unknown; items?: unknown };
    try {
      body = (await request.json()) as { tenant_id?: unknown; items?: unknown };
    } catch {
      return jsonResponse({ error: 'invalid_json' }, 400);
    }

    const tenantId = typeof body.tenant_id === 'string' ? body.tenant_id : '';
    if (!TENANT_OR_FORM_RE.test(tenantId)) {
      return jsonResponse({ error: 'invalid_tenant_id' }, 400);
    }

    if (!Array.isArray(body.items) || body.items.length === 0 || body.items.length > MAX_BATCH_ITEMS) {
      return jsonResponse({ error: 'invalid_items' }, 400);
    }

    const validItems: SpotRelayItemInput[] = [];
    for (const raw of body.items) {
      if (!raw || typeof raw !== 'object') return jsonResponse({ error: 'invalid_item' }, 400);
      const item = raw as Record<string, unknown>;
      const envelopeId = typeof item.envelope_id === 'string' ? item.envelope_id : '';
      const formId = typeof item.form_id === 'string' ? item.form_id : '';
      const envelope = typeof item.envelope === 'string' ? item.envelope : '';
      if (
        !ENVELOPE_ID_RE.test(envelopeId)
        || !TENANT_OR_FORM_RE.test(formId)
        || !envelope.startsWith('spot1:')
        || envelope.length > MAX_ENVELOPE_CHARS
      ) {
        return jsonResponse({ error: 'invalid_envelope' }, 400);
      }
      validItems.push({ envelope_id: envelopeId, form_id: formId, envelope });
    }

    const result = await store.putBatch(tenantId, validItems, Date.now());
    return jsonResponse({ ok: true, tenant_id: tenantId, ...result }, 200);
  }

  if (url.pathname === '/api/relay/v1/inbox' && request.method === 'GET') {
    if (env.SPOT_RELAY_READ_TOKEN) {
      const auth = request.headers.get('authorization') ?? '';
      if (auth !== `Bearer ${env.SPOT_RELAY_READ_TOKEN}`) {
        return jsonResponse({ error: 'unauthorized' }, 401);
      }
    }

    const tenantId = url.searchParams.get('tenant_id') ?? '';
    if (!TENANT_OR_FORM_RE.test(tenantId)) {
      return jsonResponse({ error: 'invalid_tenant_id' }, 400);
    }
    const sinceParam = Number(url.searchParams.get('since') ?? '0');
    const since = Number.isFinite(sinceParam) && sinceParam > 0 ? sinceParam : 0;
    const items = await store.list(tenantId, since);
    return jsonResponse({ ok: true, tenant_id: tenantId, items }, 200);
  }

  return jsonResponse({ error: 'not_found' }, 404);
}
