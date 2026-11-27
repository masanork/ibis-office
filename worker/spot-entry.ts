// SPDX-License-Identifier: AGPL-3.0-only
import { DurableObject } from 'cloudflare:workers';
import {
  dispatchSpotRelay,
  type SpotRelayEnv,
  type SpotRelayItemInput,
  type SpotRelayStore,
  type SpotRelayStoredItem,
} from './spot-relay';

interface SpotEntryEnv extends SpotRelayEnv {
  ASSETS: Fetcher;
  SPOT_RELAY_DO: DurableObjectNamespace<SpotTenantRelay>;
}

export class SpotTenantRelay extends DurableObject {
  private initialized = false;

  private ensureSchema(): void {
    if (this.initialized) return;
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS envelopes (
        envelope_id TEXT PRIMARY KEY,
        form_id TEXT NOT NULL,
        envelope TEXT NOT NULL,
        received_at INTEGER NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_envelopes_received_at ON envelopes(received_at);
      CREATE TABLE IF NOT EXISTS tenant_meta (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        read_token_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL
      );
    `);
    this.initialized = true;
  }

  async registerTenant(
    _tenantId: string,
    readTokenHash: string,
    now: number,
  ): Promise<'created' | 'conflict'> {
    this.ensureSchema();
    const existing = this.ctx.storage.sql
      .exec('SELECT read_token_hash FROM tenant_meta WHERE id = 1 LIMIT 1')
      .toArray();
    if (existing.length > 0) {
      return 'conflict';
    }
    this.ctx.storage.sql.exec(
      'INSERT INTO tenant_meta (id, read_token_hash, created_at) VALUES (1, ?1, ?2)',
      readTokenHash,
      now,
    );
    return 'created';
  }

  async getReadTokenHash(): Promise<string | null> {
    this.ensureSchema();
    const rows = this.ctx.storage.sql
      .exec('SELECT read_token_hash FROM tenant_meta WHERE id = 1 LIMIT 1')
      .toArray();
    if (rows.length === 0) return null;
    return String(rows[0].read_token_hash);
  }

  async putBatch(
    tenantId: string,
    items: SpotRelayItemInput[],
    now: number,
  ): Promise<{ accepted: string[]; duplicates: string[] }> {
    this.ensureSchema();
    const accepted: string[] = [];
    const duplicates: string[] = [];
    for (const item of items) {
      const existing = this.ctx.storage.sql
        .exec('SELECT 1 FROM envelopes WHERE envelope_id = ?1 LIMIT 1', item.envelope_id)
        .toArray();
      if (existing.length > 0) {
        duplicates.push(item.envelope_id);
        continue;
      }
      this.ctx.storage.sql.exec(
        'INSERT INTO envelopes (envelope_id, form_id, envelope, received_at) VALUES (?1, ?2, ?3, ?4)',
        item.envelope_id,
        item.form_id,
        item.envelope,
        now,
      );
      accepted.push(item.envelope_id);
    }
    void tenantId;
    return { accepted, duplicates };
  }

  async list(tenantId: string, since: number): Promise<SpotRelayStoredItem[]> {
    this.ensureSchema();
    const rows = this.ctx.storage.sql
      .exec(
        'SELECT envelope_id, form_id, envelope, received_at FROM envelopes WHERE received_at >= ?1 ORDER BY received_at ASC',
        since,
      )
      .toArray();
    return rows.map((row) => ({
      tenant_id: tenantId,
      envelope_id: String(row.envelope_id),
      form_id: String(row.form_id),
      envelope: String(row.envelope),
      received_at: Number(row.received_at),
    }));
  }
}

function createDurableStore(namespace: DurableObjectNamespace<SpotTenantRelay>): SpotRelayStore {
  return {
    async putBatch(tenantId, items, now) {
      const stub = namespace.get(namespace.idFromName(tenantId));
      return stub.putBatch(tenantId, items, now);
    },
    async list(tenantId, since) {
      const stub = namespace.get(namespace.idFromName(tenantId));
      return stub.list(tenantId, since);
    },
    async registerTenant(tenantId, readTokenHash, now) {
      const stub = namespace.get(namespace.idFromName(tenantId));
      return stub.registerTenant(tenantId, readTokenHash, now);
    },
    async getReadTokenHash(tenantId) {
      const stub = namespace.get(namespace.idFromName(tenantId));
      return stub.getReadTokenHash();
    },
  };
}

export default {
  async fetch(request: Request, env: SpotEntryEnv): Promise<Response> {
    const store = env.SPOT_RELAY_DO ? createDurableStore(env.SPOT_RELAY_DO) : undefined;
    const relayResponse = await dispatchSpotRelay(request, env, store);
    if (relayResponse) return relayResponse;
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler<SpotEntryEnv>;
