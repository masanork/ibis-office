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
    `);
    this.initialized = true;
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
