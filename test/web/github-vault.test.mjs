// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
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
