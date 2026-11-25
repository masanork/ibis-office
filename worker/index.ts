// SPDX-License-Identifier: AGPL-3.0-only
import { dispatchGitHubVault } from './github-vault';
import { dispatchSoraneAdmin } from './sorane-gateway';
import { serveBrotliWasm } from './wasm-delivery';

export default {
  async fetch(request: Request, env: CloudflareEnv): Promise<Response> {
    const wasmResponse = await serveBrotliWasm(request, env.ASSETS);
    if (wasmResponse) return wasmResponse;
    const githubResponse = await dispatchGitHubVault(request, env as Record<string, string | undefined>);
    if (githubResponse) return githubResponse;
    const adminResponse = await dispatchSoraneAdmin(request, env.SORANE_ADMIN);
    if (adminResponse) return adminResponse;
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler<CloudflareEnv>;
