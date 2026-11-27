// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  detectSoraneContentDir,
  dispatchGitHubVault,
  isGitHubVaultRoute,
  sealSession,
  unsealSession,
} from '../../worker/github-vault.js';

const TEST_ENV = {
  GITHUB_APP_CLIENT_ID: 'Iv1.testclientid',
  GITHUB_APP_CLIENT_SECRET: 'test-client-secret',
  GITHUB_APP_SLUG: 'soranedocs-vault',
  GITHUB_SESSION_SECRET: 'super-secret-session-key-for-tests',
};

test('isGitHubVaultRoute matches /auth/github/* and /api/github/* only', () => {
  assert.equal(isGitHubVaultRoute('/auth/github/login'), true);
  assert.equal(isGitHubVaultRoute('/api/github/session'), true);
  assert.equal(isGitHubVaultRoute('/api/samples'), false);
  assert.equal(isGitHubVaultRoute('/index.html'), false);
});

test('sealSession and unsealSession round-trip and reject tampered tokens', async () => {
  const payload = {
    accessToken: 'ghu_secret_token',
    login: 'masanork',
    vault: { owner: 'sorane-dev', repo: 'notes', branch: 'main', rootPath: 'docs' },
    iat: 1700000000000,
  };
  const token = await sealSession(payload, TEST_ENV.GITHUB_SESSION_SECRET);
  assert.ok(!token.includes('ghu_secret_token'), 'token must be encrypted');
  const decoded = await unsealSession(token, TEST_ENV.GITHUB_SESSION_SECRET);
  assert.deepEqual(decoded, payload);
  assert.equal(await unsealSession(token, 'wrong-secret'), null);
  assert.equal(await unsealSession(token.slice(0, -4) + 'AAAA', TEST_ENV.GITHUB_SESSION_SECRET), null);
});

test('GET /api/github/session reports unconfigured state safely', async () => {
  const res = await dispatchGitHubVault(new Request('https://docs.sorane.dev/api/github/session'), {});
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.deepEqual(body, {
    configured: false,
    loggedIn: false,
    login: null,
    vault: null,
    installUrl: null,
  });
});

test('GET /auth/github/login redirects to GitHub OAuth with state cookie', async () => {
  const res = await dispatchGitHubVault(new Request('https://docs.sorane.dev/auth/github/login'), TEST_ENV);
  assert.equal(res.status, 302);
  const location = new URL(res.headers.get('Location'));
  assert.equal(location.origin, 'https://github.com');
  assert.equal(location.pathname, '/login/oauth/authorize');
  assert.equal(location.searchParams.get('client_id'), TEST_ENV.GITHUB_APP_CLIENT_ID);
  assert.equal(location.searchParams.get('redirect_uri'), 'https://docs.sorane.dev/auth/github/callback');
  const cookie = res.headers.get('Set-Cookie') || '';
  assert.match(cookie, /sorane_gh_state=/);
  assert.match(cookie, /HttpOnly/);
  assert.match(cookie, /Secure/);
});

test('GET /api/github/session returns login and vault without exposing accessToken', async () => {
  const sealed = await sealSession(
    {
      accessToken: 'ghu_never_expose',
      login: 'masanork',
      vault: { owner: 'sorane-dev', repo: 'vault', branch: 'main', rootPath: '' },
      iat: Date.now(),
    },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const res = await dispatchGitHubVault(
    new Request('https://docs.sorane.dev/api/github/session', {
      headers: { Cookie: `sorane_gh_session=${sealed}` },
    }),
    TEST_ENV,
  );
  assert.equal(res.status, 200);
  const text = await res.text();
  assert.ok(!text.includes('ghu_never_expose'));
  const body = JSON.parse(text);
  assert.equal(body.configured, true);
  assert.equal(body.loggedIn, true);
  assert.equal(body.login, 'masanork');
  assert.equal(body.vault.owner, 'sorane-dev');
  assert.equal(body.installUrl, 'https://github.com/apps/soranedocs-vault/installations/new');
});

test('POST /api/github/vault rejects path traversal and unauthenticated requests', async () => {
  const unauth = await dispatchGitHubVault(
    new Request('https://docs.sorane.dev/api/github/vault', { method: 'POST', body: '{}' }),
    TEST_ENV,
  );
  assert.equal(unauth.status, 401);

  const sealed = await sealSession(
    { accessToken: 'ghu_tok', login: 'masanork', vault: null, iat: Date.now() },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const badOwner = await dispatchGitHubVault(
    new Request('https://docs.sorane.dev/api/github/vault', {
      method: 'POST',
      headers: { Cookie: `sorane_gh_session=${sealed}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({ owner: '../evil', repo: 'test' }),
    }),
    TEST_ENV,
  );
  assert.equal(badOwner.status, 400);
});

test('POST /api/github/repos/create validates auth and repository name', async () => {
  const unauth = await dispatchGitHubVault(
    new Request('https://docs.sorane.dev/api/github/repos/create', { method: 'POST', body: '{}' }),
    TEST_ENV,
  );
  assert.equal(unauth.status, 401);

  const sealed = await sealSession(
    { accessToken: 'ghu_tok', login: 'masanork', vault: null, iat: Date.now() },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const badName = await dispatchGitHubVault(
    new Request('https://docs.sorane.dev/api/github/repos/create', {
      method: 'POST',
      headers: { Cookie: `sorane_gh_session=${sealed}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: '../bad name' }),
    }),
    TEST_ENV,
  );
  assert.equal(badName.status, 400);
});

test('POST /api/github/repos/create includes permission diagnostics on 403', async () => {
  const sealed = await sealSession(
    { accessToken: 'ghu_tok', login: 'masanork', vault: null, iat: Date.now() },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const origFetch = globalThis.fetch;
  globalThis.fetch = async (input) => {
    const url = String(input);
    if (url.endsWith('/user/repos')) {
      return new Response(JSON.stringify({ message: 'Resource not accessible by integration' }), {
        status: 403,
        headers: {
          'Content-Type': 'application/json',
          'X-Accepted-GitHub-Permissions': 'administration=write',
        },
      });
    }
    if (url.includes('/user/installations')) {
      return new Response(
        JSON.stringify({
          installations: [
            {
              account: { login: 'masanork' },
              repository_selection: 'selected',
              permissions: { contents: 'write', repository_creation: 'write' },
            },
          ],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    throw new Error(`Unexpected fetch: ${url}`);
  };
  try {
    const res = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/github/repos/create', {
        method: 'POST',
        headers: { Cookie: `sorane_gh_session=${sealed}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'soraneDocs' }),
      }),
      TEST_ENV,
    );
    assert.equal(res.status, 403);
    const body = await res.json();
    assert.match(body.error, /required: administration=write/);
    assert.match(body.error, /selection: selected/);
    assert.match(body.error, /granted: contents:write, repository_creation:write/);
    assert.equal(body.diagnostics.acceptedPermissions, 'administration=write');
    assert.equal(body.diagnostics.installation.repositorySelection, 'selected');
  } finally {
    globalThis.fetch = origFetch;
  }
});

test('GET /api/github/repos reports installed and canCreateRepo flags', async () => {
  const sealed = await sealSession(
    { accessToken: 'ghu_tok', login: 'masanork', vault: null, iat: Date.now() },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const origFetch = globalThis.fetch;
  globalThis.fetch = async (input) => {
    const url = String(input);
    if (url.includes('/user/installations')) {
      return new Response(JSON.stringify({ installations: [] }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (url.includes('/user/repos')) {
      return new Response(JSON.stringify([]), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    throw new Error(`Unexpected fetch: ${url}`);
  };
  try {
    const res = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/github/repos', {
        headers: { Cookie: `sorane_gh_session=${sealed}` },
      }),
      TEST_ENV,
    );
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.installed, false);
    assert.equal(body.canCreateRepo, false);
    assert.equal(body.installUrl, 'https://github.com/apps/soranedocs-vault/installations/new');
  } finally {
    globalThis.fetch = origFetch;
  }
});

test('GET /api/github/repos requires repository_selection=all on personal installation for canCreateRepo', async () => {
  const sealed = await sealSession(
    { accessToken: 'ghu_tok', login: 'masanork', vault: null, iat: Date.now() },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const origFetch = globalThis.fetch;
  let selection = 'selected';
  globalThis.fetch = async (input) => {
    const url = String(input);
    if (url.includes('/user/installations?')) {
      return new Response(
        JSON.stringify({
          installations: [
            {
              id: 42,
              account: { login: 'masanork' },
              repository_selection: selection,
              html_url: 'https://github.com/settings/installations/42',
              permissions: { contents: 'write', repository_creation: 'write' },
            },
          ],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (url.includes('/user/installations/42/repositories')) {
      return new Response(JSON.stringify({ repositories: [] }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (url.includes('/user/repos')) {
      return new Response(JSON.stringify([]), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    throw new Error(`Unexpected fetch: ${url}`);
  };
  try {
    const selectedRes = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/github/repos', {
        headers: { Cookie: `sorane_gh_session=${sealed}` },
      }),
      TEST_ENV,
    );
    assert.equal(selectedRes.status, 200);
    const selectedBody = await selectedRes.json();
    assert.equal(selectedBody.installed, true);
    assert.equal(selectedBody.canCreateRepo, false);
    assert.equal(selectedBody.repoSelection, 'selected');
    assert.equal(selectedBody.installSettingsUrl, 'https://github.com/settings/installations/42');

    selection = 'all';
    const allRes = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/github/repos', {
        headers: { Cookie: `sorane_gh_session=${sealed}` },
      }),
      TEST_ENV,
    );
    assert.equal(allRes.status, 200);
    const allBody = await allRes.json();
    assert.equal(allBody.installed, true);
    assert.equal(allBody.canCreateRepo, true);
    assert.equal(allBody.repoSelection, 'all');
  } finally {
    globalThis.fetch = origFetch;
  }
});

test('POST /api/github/repos/create fails with initFailed when index.md commit is rejected', async () => {
  const sealed = await sealSession(
    { accessToken: 'ghu_tok', login: 'masanork', vault: null, iat: Date.now() },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const origFetch = globalThis.fetch;
  globalThis.fetch = async (input, init) => {
    const url = String(input);
    if (url.endsWith('/user/repos') && init?.method === 'POST') {
      return new Response(
        JSON.stringify({
          name: 'soraneDocs',
          owner: { login: 'masanork' },
          default_branch: 'main',
          html_url: 'https://github.com/masanork/soraneDocs',
        }),
        { status: 201, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (url.includes('/contents/index.md') && init?.method === 'PUT') {
      return new Response(JSON.stringify({ message: 'Resource not accessible by integration' }), {
        status: 403,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (url.includes('/user/installations')) {
      return new Response(
        JSON.stringify({
          installations: [
            {
              account: { login: 'masanork' },
              html_url: 'https://github.com/settings/installations/42',
            },
          ],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    throw new Error(`Unexpected fetch: ${url}`);
  };
  try {
    const res = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/github/repos/create', {
        method: 'POST',
        headers: { Cookie: `sorane_gh_session=${sealed}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'soraneDocs' }),
      }),
      TEST_ENV,
    );
    assert.equal(res.status, 409);
    const body = await res.json();
    assert.equal(body.initFailed, true);
    assert.equal(body.repoUrl, 'https://github.com/masanork/soraneDocs');
    assert.equal(body.installSettingsUrl, 'https://github.com/settings/installations/42');
  } finally {
    globalThis.fetch = origFetch;
  }
});

test('POST /api/github/repos/create succeeds with structured OKF help index.md and sets vault cookie', async () => {
  const sealed = await sealSession(
    { accessToken: 'ghu_tok', login: 'masanork', vault: null, iat: Date.now() },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const origFetch = globalThis.fetch;
  let capturedIndexMarkdown = '';
  globalThis.fetch = async (input, init) => {
    const url = String(input);
    if (url.endsWith('/user/repos') && init?.method === 'POST') {
      return new Response(
        JSON.stringify({
          name: 'soraneDocs',
          owner: { login: 'masanork' },
          default_branch: 'main',
          html_url: 'https://github.com/masanork/soraneDocs',
        }),
        { status: 201, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (url.includes('/contents/index.md') && init?.method === 'PUT') {
      const payload = JSON.parse(String(init.body));
      capturedIndexMarkdown = Buffer.from(payload.content, 'base64').toString('utf8');
      return new Response(JSON.stringify({ content: { sha: 'abc123' } }), {
        status: 201,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    throw new Error(`Unexpected fetch: ${url}`);
  };
  try {
    const res = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/github/repos/create', {
        method: 'POST',
        headers: { Cookie: `sorane_gh_session=${sealed}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'soraneDocs' }),
      }),
      TEST_ENV,
    );
    assert.equal(res.status, 201);
    const body = await res.json();
    assert.equal(body.ok, true);
    assert.deepEqual(body.vault, { owner: 'masanork', repo: 'soraneDocs', branch: 'main', rootPath: '' });
    assert.match(res.headers.get('Set-Cookie') || '', /sorane_gh_session=/);
    assert.match(capturedIndexMarkdown, /profile: sorane-okf\/0\.1/);
    assert.match(capturedIndexMarkdown, /## 1\. 基本的な使い方/);
    assert.match(capturedIndexMarkdown, /## 2\. 対応しているファイル形式/);
    assert.match(capturedIndexMarkdown, /## 3\. おすすめのフォルダ構成/);
  } finally {
    globalThis.fetch = origFetch;
  }
});

test('POST /api/github/repos/create returns alreadyExists guidance on 422', async () => {
  const sealed = await sealSession(
    { accessToken: 'ghu_tok', login: 'masanork', vault: null, iat: Date.now() },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const origFetch = globalThis.fetch;
  globalThis.fetch = async (input, init) => {
    const url = String(input);
    if (url.endsWith('/user/repos') && init?.method === 'POST') {
      return new Response(
        JSON.stringify({ message: 'Repository creation failed.' }),
        { status: 422, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (url.includes('/user/installations')) {
      return new Response(
        JSON.stringify({
          installations: [
            {
              account: { login: 'masanork' },
              html_url: 'https://github.com/settings/installations/42',
            },
          ],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    throw new Error(`Unexpected fetch: ${url}`);
  };
  try {
    const res = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/github/repos/create', {
        method: 'POST',
        headers: { Cookie: `sorane_gh_session=${sealed}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'soraneDocs' }),
      }),
      TEST_ENV,
    );
    assert.equal(res.status, 422);
    const body = await res.json();
    assert.equal(body.alreadyExists, true);
    assert.equal(body.installSettingsUrl, 'https://github.com/settings/installations/42');
    assert.match(body.error, /すでに存在するため/);
  } finally {
    globalThis.fetch = origFetch;
  }
});

test('GET /api/ai/models and POST /api/ai/chat dynamically discover and proxy GitHub Copilot API models', async () => {
  const unauth = await dispatchGitHubVault(
    new Request('https://docs.sorane.dev/api/ai/models'),
    TEST_ENV,
  );
  assert.equal(unauth.status, 401);

  const sealed = await sealSession(
    { accessToken: 'ghu_ai_tok', login: 'masanork', vault: null, iat: 1700000000000 },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );

  const origFetch = globalThis.fetch;
  const seenChatModels = [];
  globalThis.fetch = async (input, init) => {
    const url = String(input);
    if (url === 'https://api.github.com/copilot_internal/v2/token') {
      assert.equal(init.headers.Authorization, 'Bearer ghu_ai_tok');
      assert.ok(init.headers['Copilot-Integration-Id']);
      return new Response(
        JSON.stringify({
          token: 'tid=copilot_session_tok',
          expires_at: 1900000000,
          endpoints: { api: 'https://api.individual.githubcopilot.com' },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (url === 'https://api.individual.githubcopilot.com/models') {
      assert.equal(init.headers.Authorization, 'Bearer tid=copilot_session_tok');
      return new Response(
        JSON.stringify({
          object: 'list',
          data: [
            {
              id: 'text-embedding-3-small',
              capabilities: { type: 'embeddings' },
            },
            {
              id: 'disabled-chat',
              policy: { state: 'disabled' },
              capabilities: { type: 'chat' },
            },
            {
              id: 'gpt-4.1',
              name: 'GPT-4.1',
              vendor: 'OpenAI',
              model_picker_enabled: true,
              capabilities: {
                type: 'chat',
                limits: { max_prompt_tokens: 64000 },
                supports: { streaming: true, tool_calls: true },
              },
            },
            {
              id: 'gpt-5-mini',
              name: 'GPT-5 mini',
              vendor: 'OpenAI',
              model_picker_enabled: true,
              capabilities: {
                type: 'chat',
                limits: { max_prompt_tokens: 32000 },
                supports: { streaming: true, tool_calls: true },
              },
            },
          ],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (url === 'https://api.individual.githubcopilot.com/chat/completions') {
      assert.equal(init.headers.Authorization, 'Bearer tid=copilot_session_tok');
      assert.equal(init.headers['Openai-Intent'], 'conversation-panel');
      const payload = JSON.parse(String(init.body));
      seenChatModels.push(payload.model);
      return new Response('data: {"choices":[{"delta":{"content":"hello"}}]}\n\ndata: [DONE]\n\n', {
        status: 200,
        headers: { 'Content-Type': 'text/event-stream' },
      });
    }
    throw new Error(`Unexpected fetch: ${url}`);
  };

  try {
    const modelsRes = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/ai/models', {
        headers: { Cookie: `sorane_gh_session=${sealed}` },
      }),
      TEST_ENV,
    );
    assert.equal(modelsRes.status, 200);
    const catalog = await modelsRes.json();
    assert.ok(typeof catalog.csrf === 'string' && catalog.csrf.length > 10);
    assert.equal(catalog.provider, 'github-copilot');
    assert.deepEqual(
      catalog.models.map((m) => m.key),
      ['gpt-4.1', 'gpt-5-mini'],
    );
    assert.equal(catalog.defaultModel, 'gpt-4.1');
    assert.equal(catalog.completionModel, 'gpt-5-mini');

    const badCsrf = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/ai/chat', {
        method: 'POST',
        headers: {
          Cookie: `sorane_gh_session=${sealed}`,
          'Content-Type': 'application/json',
          'X-Sorane-CSRF': 'wrong',
        },
        body: JSON.stringify({ model: 'gpt-4.1', messages: [{ role: 'user', content: 'hi' }] }),
      }),
      TEST_ENV,
    );
    assert.equal(badCsrf.status, 403);

    // Explicitly requested available model
    const chatRes = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/ai/chat', {
        method: 'POST',
        headers: {
          Cookie: `sorane_gh_session=${sealed}`,
          'Content-Type': 'application/json',
          'X-Sorane-CSRF': catalog.csrf,
        },
        body: JSON.stringify({ model: 'gpt-4.1', messages: [{ role: 'user', content: 'hi' }] }),
      }),
      TEST_ENV,
    );
    assert.equal(chatRes.status, 200);
    assert.equal(chatRes.headers.get('X-Sorane-Model'), 'gpt-4.1');
    const sseText = await chatRes.text();
    assert.match(sseText, /"content":"hello"/);

    // Stale/unlisted model for completion falls back to dynamically discovered fast completionModel
    const completionRes = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/ai/chat', {
        method: 'POST',
        headers: {
          Cookie: `sorane_gh_session=${sealed}`,
          'Content-Type': 'application/json',
          'X-Sorane-CSRF': catalog.csrf,
        },
        body: JSON.stringify({
          model: 'openai/gpt-4.1-mini',
          purpose: 'completion',
          messages: [{ role: 'user', content: 'complete this' }],
        }),
      }),
      TEST_ENV,
    );
    assert.equal(completionRes.status, 200);
    assert.equal(completionRes.headers.get('X-Sorane-Model'), 'gpt-5-mini');
    assert.deepEqual(seenChatModels, ['gpt-4.1', 'gpt-5-mini']);
  } finally {
    globalThis.fetch = origFetch;
  }
});

test('detectSoraneContentDir extracts build.content_dir from sorane.yaml', () => {
  assert.equal(
    detectSoraneContentDir('site:\n  title: Demo\nbuild:\n  content_dir: "docs/content"\n'),
    'docs/content',
  );
  assert.equal(
    detectSoraneContentDir('site:\n  title: Demo\nbuild:\n  out_dir: dist\n'),
    'content',
  );
  assert.equal(
    detectSoraneContentDir('build:\n  content_dir: ../escape\n'),
    'content',
  );
});

test('POST /api/github/vault auto-detects build.content_dir from sorane.yaml when rootPath is omitted', async () => {
  const sealed = await sealSession(
    { accessToken: 'ghu_tok', login: 'masanork', vault: null, iat: Date.now() },
    TEST_ENV.GITHUB_SESSION_SECRET,
  );
  const yaml = 'site:\n  title: Demo\nbuild:\n  content_dir: docs/pages\n';
  const origFetch = globalThis.fetch;
  globalThis.fetch = async (input) => {
    const url = String(input);
    if (url.endsWith('/repos/masanork/site')) {
      return new Response(JSON.stringify({ default_branch: 'main' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (url.endsWith('/repos/masanork/site/contents/sorane.yaml?ref=main')) {
      return new Response(
        JSON.stringify({ encoding: 'base64', content: Buffer.from(yaml, 'utf8').toString('base64') }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    throw new Error(`Unexpected fetch: ${url}`);
  };
  try {
    const res = await dispatchGitHubVault(
      new Request('https://docs.sorane.dev/api/github/vault', {
        method: 'POST',
        headers: { Cookie: `sorane_gh_session=${sealed}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ owner: 'masanork', repo: 'site' }),
      }),
      TEST_ENV,
    );
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.ok, true);
    assert.equal(body.vault.rootPath, 'docs/pages');
    assert.equal(body.detectedSoraneContentDir, 'docs/pages');
  } finally {
    globalThis.fetch = origFetch;
  }
});
