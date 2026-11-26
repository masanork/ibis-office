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
  registerTenant(tenantId: string, readTokenHash: string, now: number): Promise<'created' | 'conflict'>;
  getReadTokenHash(tenantId: string): Promise<string | null>;
}

export interface SpotRelayEnv {
  SPOT_RELAY_READ_TOKEN?: string;
}

const TENANT_OR_FORM_RE = /^[a-zA-Z0-9._-]{1,64}$/;
const HEX_SHA256_RE = /^[a-f0-9]{64}$/;
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

async function sha256Hex(text: string): Promise<string> {
  const bytes = new TextEncoder().encode(text);
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return Array.from(digest, (b) => b.toString(16).padStart(2, '0')).join('');
}

function timingSafeEqualHex(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

export function createMemoryRelayStore(): SpotRelayStore {
  const byTenant = new Map<string, Map<string, SpotRelayStoredItem>>();
  const tenantHashes = new Map<string, string>();
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
    async registerTenant(tenantId, readTokenHash) {
      if (tenantHashes.has(tenantId)) return 'conflict';
      tenantHashes.set(tenantId, readTokenHash);
      return 'created';
    },
    async getReadTokenHash(tenantId) {
      return tenantHashes.get(tenantId) ?? null;
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

  if (url.pathname === '/api/relay/v1/tenants' && request.method === 'POST') {
    let body: { tenant_id?: unknown; read_token_hash?: unknown };
    try {
      body = (await request.json()) as { tenant_id?: unknown; read_token_hash?: unknown };
    } catch {
      return jsonResponse({ error: 'invalid_json' }, 400);
    }

    const tenantId = typeof body.tenant_id === 'string' ? body.tenant_id : '';
    const readTokenHash = typeof body.read_token_hash === 'string' ? body.read_token_hash.toLowerCase() : '';
    if (!TENANT_OR_FORM_RE.test(tenantId)) {
      return jsonResponse({ error: 'invalid_tenant_id' }, 400);
    }
    if (!HEX_SHA256_RE.test(readTokenHash)) {
      return jsonResponse({ error: 'invalid_read_token_hash' }, 400);
    }

    const outcome = await store.registerTenant(tenantId, readTokenHash, Date.now());
    if (outcome === 'conflict') {
      return jsonResponse({ error: 'tenant_already_registered' }, 409);
    }
    return jsonResponse({ ok: true, tenant_id: tenantId }, 201);
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
        !HEX_SHA256_RE.test(envelopeId)
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
    const tenantId = url.searchParams.get('tenant_id') ?? '';
    if (!TENANT_OR_FORM_RE.test(tenantId)) {
      return jsonResponse({ error: 'invalid_tenant_id' }, 400);
    }

    const tenantHash = await store.getReadTokenHash(tenantId);
    const auth = request.headers.get('authorization') ?? '';
    const bearer = auth.startsWith('Bearer ') ? auth.slice(7) : '';

    if (tenantHash) {
      if (!bearer) {
        return jsonResponse({ error: 'unauthorized' }, 401);
      }
      const suppliedHash = await sha256Hex(bearer);
      if (!timingSafeEqualHex(suppliedHash, tenantHash)) {
        return jsonResponse({ error: 'unauthorized' }, 401);
      }
    } else if (env.SPOT_RELAY_READ_TOKEN) {
      if (bearer !== env.SPOT_RELAY_READ_TOKEN) {
        return jsonResponse({ error: 'unauthorized' }, 401);
      }
    }

    const sinceParam = Number(url.searchParams.get('since') ?? '0');
    const since = Number.isFinite(sinceParam) && sinceParam > 0 ? sinceParam : 0;
    const items = await store.list(tenantId, since);
    return jsonResponse({ ok: true, tenant_id: tenantId, items }, 200);
  }

  return jsonResponse({ error: 'not_found' }, 404);
}
