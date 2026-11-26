// SPDX-License-Identifier: AGPL-3.0-only
import { dispatchSpotRelay, type SpotRelayEnv } from './spot-relay';

interface SpotEntryEnv extends SpotRelayEnv {
  ASSETS: Fetcher;
}

export default {
  async fetch(request: Request, env: SpotEntryEnv): Promise<Response> {
    const relayResponse = await dispatchSpotRelay(request, env);
    if (relayResponse) return relayResponse;
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler<SpotEntryEnv>;
