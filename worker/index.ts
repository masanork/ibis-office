// SPDX-License-Identifier: AGPL-3.0-only
import { dispatchGitHubVault, type GitHubVaultEnv } from './github-vault';
import { serveBrotliWasm } from './wasm-delivery';
import { serveServiceWorker } from './sw-headers';
import { dispatchVcDirectPost } from './vc-direct-post';

export default {
  async fetch(request: Request, env: CloudflareEnv): Promise<Response> {
    const swResponse = await serveServiceWorker(request, env.ASSETS);
    if (swResponse) return swResponse;
    const wasmResponse = await serveBrotliWasm(request, env.ASSETS);
    if (wasmResponse) return wasmResponse;
    const vcDirectPostResponse = await dispatchVcDirectPost(request);
    if (vcDirectPostResponse) return vcDirectPostResponse;
    const githubResponse = await dispatchGitHubVault(request, env as unknown as GitHubVaultEnv);
    if (githubResponse) return githubResponse;
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler<CloudflareEnv>;
