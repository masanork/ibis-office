// SPDX-License-Identifier: AGPL-3.0-only
import { dispatchGitHubVault, type GitHubVaultEnv } from './github-vault';
import { serveBrotliWasm } from './wasm-delivery';

export default {
  async fetch(request: Request, env: CloudflareEnv): Promise<Response> {
    const wasmResponse = await serveBrotliWasm(request, env.ASSETS);
    if (wasmResponse) return wasmResponse;
    const githubResponse = await dispatchGitHubVault(request, env as unknown as GitHubVaultEnv);
    if (githubResponse) return githubResponse;
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler<CloudflareEnv>;
