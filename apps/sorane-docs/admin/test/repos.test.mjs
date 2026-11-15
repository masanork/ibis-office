// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { createHash, webcrypto } from 'node:crypto';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { generateKeyPair, exportJWK } from 'jose';

globalThis.crypto ??= webcrypto;
const dir = await mkdtemp(join(tmpdir(), 'sorane-repos-test-'));
await build({
  entryPoints: ['src/index.ts'],
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  outfile: join(dir, 'worker.mjs'),
});
await build({
  entryPoints: ['src/git-pack.ts'],
  bundle: true,
  platform: 'node',
  format: 'esm',
  outfile: join(dir, 'git-pack.mjs'),
});
const { default: worker } = await import(pathToFileURL(join(dir, 'worker.mjs')));
const gitPack = await import(pathToFileURL(join(dir, 'git-pack.mjs')));

const { privateKey } = await generateKeyPair('ES256', { extractable: true });
const jwk = await exportJWK(privateKey);
jwk.kid = 'test-only';

const host = 'https://docs.example';
const issuer = 'https://auth.example';
const digest = (s) => createHash('sha256').update(s).digest('base64url');
const browser = 'browser-repos-test';
const csrf = digest(browser);
const now = () => Math.floor(Date.now() / 1000);
const session = (token, sub) => ({
  token_hash: digest(token),
  sid: `${sub}-sid`,
  sub,
  auth_time: 123,
  lease_until: now() + 100,
  parent_expires_at: now() + 3600,
  idle_expires_at: now() + 600,
  idle_timeout_seconds: 600,
});

function createMockEnv() {
  const docRepos = new Map();
  const artifactsRepos = new Map();
  let seq = 0;

  const matchSql = (sql, start) => sql.replace(/\s+/g, ' ').trim().startsWith(start);
  const db = {
    withSession: () => db,
    async batch(statements) {
      const results = [];
      for (const s of statements) results.push(await s.run());
      return results;
    },
    prepare(sql) {
      let args = [];
      return {
        bind(...v) {
          args = v;
          return this;
        },
        async first() {
          if (matchSql(sql, 'SELECT * FROM rp_session')) {
            if (args[0] === digest('owner-token')) return session('owner-token', 'owner-sub');
            if (args[0] === digest('other-token')) return session('other-token', 'other-sub');
            return null;
          }
          if (matchSql(sql, 'UPDATE rp_session')) {
            const token = args[7];
            if (token === digest('owner-token')) return session('owner-token', 'owner-sub');
            if (token === digest('other-token')) return session('other-token', 'other-sub');
            return null;
          }
          if (matchSql(sql, 'SELECT id,issuer,sub,repo_name,slug,title,description,default_branch,created_at,updated_at FROM doc_repos')) {
            const row = docRepos.get(args[0]);
            return row && row.issuer === args[1] && row.sub === args[2] ? { ...row } : null;
          }
          throw new Error('Unexpected first SQL: ' + sql);
        },
        async all() {
          if (matchSql(sql, 'SELECT id,repo_name,slug,title,description,default_branch,created_at,updated_at FROM doc_repos') || matchSql(sql, 'SELECT id,slug,title,description,default_branch,updated_at FROM doc_repos')) {
            return {
              results: [...docRepos.values()]
                .filter((r) => r.issuer === args[0] && r.sub === args[1])
                .sort((a, b) => b.updated_at - a.updated_at),
            };
          }
          throw new Error('Unexpected all SQL: ' + sql);
        },
        async run() {
          if (matchSql(sql, 'INSERT INTO doc_repos')) {
            docRepos.set(args[0], {
              id: args[0],
              issuer: args[1],
              sub: args[2],
              repo_name: args[3],
              slug: args[4],
              title: args[5],
              description: args[6],
              default_branch: args[7],
              created_at: args[8],
              updated_at: args[9],
            });
            return { success: true, meta: { changes: 1 } };
          }
          if (matchSql(sql, 'UPDATE doc_repos SET updated_at=?')) {
            const row = docRepos.get(args[1]);
            if (!row || row.issuer !== args[2] || row.sub !== args[3]) {
              return { success: true, meta: { changes: 0 } };
            }
            row.updated_at = args[0];
            return { success: true, meta: { changes: 1 } };
          }
          if (matchSql(sql, 'DELETE FROM doc_repos')) {
            const row = docRepos.get(args[0]);
            if (!row || row.issuer !== args[1] || row.sub !== args[2]) {
              return { success: true, meta: { changes: 0 } };
            }
            docRepos.delete(args[0]);
            return { success: true, meta: { changes: 1 } };
          }
          throw new Error('Unexpected run SQL: ' + sql);
        },
      };
    },
  };

  function makeRepoState(name, opts = {}) {
    return {
      id: `repo-${++seq}`,
      name,
      description: opts.description ?? null,
      defaultBranch: opts.setDefaultBranch ?? 'main',
      remote: `https://artifacts.example/docs/${name}.git`,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      lastPushAt: null,
      source: opts.source ?? null,
      readOnly: false,
      commits: [],
      trees: new Map(), // treeHash -> Map(name -> entry)
      blobs: new Map(), // blobHash -> Uint8Array
      files: new Map(), // path -> Uint8Array
      tokens: new Map(),
    };
  }

  function makeHandle(state) {
    return {
      [Symbol.dispose]() {},
      async info() {
        return {
          id: state.id,
          name: state.name,
          description: state.description,
          defaultBranch: state.defaultBranch,
          createdAt: state.createdAt,
          updatedAt: state.updatedAt,
          lastPushAt: state.lastPushAt,
          source: state.source,
          readOnly: state.readOnly,
          remote: state.remote,
        };
      },
      async log(opts = {}) {
        const limit = opts.limit ?? 50;
        return [...state.commits].reverse().slice(0, limit);
      },
      async readTree(treeHash) {
        const entries = state.trees.get(treeHash);
        return entries ? [...entries.values()] : null;
      },
      async readBlob(hash) {
        const b = state.blobs.get(hash);
        return b ? new Blob([b]) : null;
      },
      async readCommit(hash) {
        return state.commits.find((c) => c.hash === hash) ?? null;
      },
      async readFile({ path }) {
        const b = state.files.get(path);
        return b ? new Blob([b]) : null;
      },
      async createToken(scope = 'write', ttl = 86400) {
        const id = `tok-${++seq}`;
        const plaintext = `art_${scope}_${id}`;
        const info = {
          id,
          scope,
          state: 'active',
          createdAt: new Date().toISOString(),
          expiresAt: new Date(Date.now() + ttl * 1000).toISOString(),
        };
        state.tokens.set(id, { ...info, plaintext });
        return { id, plaintext, scope, expiresAt: info.expiresAt };
      },
      async listTokens() {
        const tokens = [...state.tokens.values()].map(({ plaintext: _p, ...rest }) => rest);
        return { tokens, total: tokens.length };
      },
      async revokeToken(tokenOrId) {
        return state.tokens.delete(tokenOrId);
      },
      async fork(targetName, opts = {}) {
        if (artifactsRepos.has(targetName)) throw new Error('ALREADY_EXISTS');
        const forked = makeRepoState(targetName, {
          description: opts.description ?? state.description,
          setDefaultBranch: state.defaultBranch,
          source: `artifacts:docs/${state.name}`,
        });
        forked.commits = [...state.commits];
        forked.trees = new Map(state.trees);
        forked.blobs = new Map(state.blobs);
        forked.files = new Map(state.files);
        artifactsRepos.set(targetName, forked);
        return {
          id: forked.id,
          name: forked.name,
          description: forked.description,
          defaultBranch: forked.defaultBranch,
          remote: forked.remote,
          token: 'fork-tok',
        };
      },
    };
  }

  const artifacts = {
    async create(name, opts = {}) {
      const state = makeRepoState(name, opts);
      artifactsRepos.set(name, state);
      return {
        id: state.id,
        name: state.name,
        description: state.description,
        defaultBranch: state.defaultBranch,
        remote: state.remote,
        token: 'init-tok',
      };
    },
    async get(name) {
      const state = artifactsRepos.get(name);
      if (!state) throw new Error('NOT_FOUND');
      return makeHandle(state);
    },
    async delete(name) {
      return artifactsRepos.delete(name);
    },
  };

  const env = {
    ISSUER: issuer,
    RP_ORIGIN: host,
    CLIENT_ID: 'test-client',
    RP_PRIVATE_JWK: JSON.stringify(jwk),
    AUTH_LIMITER: { limit: async () => ({ success: true }) },
    DB: db,
    FILES: {},
    SAMPLES: {},
    ARTIFACTS: artifacts,
  };

  return { env, artifactsRepos, docRepos };
}

// Intercept fetch to simulate Artifacts git-receive-pack remote
function installGitFetchMock(artifactsRepos) {
  const origFetch = globalThis.fetch;
  globalThis.fetch = async (input, init) => {
    const urlStr = typeof input === 'string' ? input : input.url;
    const match = /^https:\/\/artifacts\.example\/docs\/([^/]+)\.git\/git-receive-pack$/.exec(urlStr);
    if (!match) return origFetch(input, init);

    const state = artifactsRepos.get(match[1]);
    if (!state) return new Response('Not found', { status: 404 });

    const body = new Uint8Array(await new Response(init.body).arrayBuffer());
    const packOffset = new TextDecoder().decode(body).indexOf('PACK');
    assert.ok(packOffset >= 0, 'git-receive-pack payload must contain PACK');
    const pack = body.subarray(packOffset);
    const view = new DataView(pack.buffer, pack.byteOffset, pack.byteLength);
    assert.equal(view.getUint32(4, false), 2, 'PACK version 2');
    const objCount = view.getUint32(8, false);
    assert.ok(objCount >= 2, 'PACK must contain at least tree and commit');

    // Parse objects from PACK (decompress each zlib stream)
    let pos = 12;
    const endPos = pack.length - 20;
    const parsedObjects = [];
    for (let i = 0; i < objCount; i++) {
      let byte = pack[pos++];
      const typeNum = (byte >> 4) & 0x07;
      while (byte & 0x80) byte = pack[pos++];
      // Find next zlib stream by decompressing from pos
      const slice = pack.subarray(pos, endPos);
      // In our packfile, each deflate stream ends cleanly; use Node zlib inflateSync
      const { inflateSync } = await import('node:zlib');
      const res = inflateSync(slice, { info: true });
      pos += res.engine.bytesWritten;
      parsedObjects.push({ typeNum, data: new Uint8Array(res.buffer) });
    }

    // Store blobs, trees, commits into mock repo state
    for (const obj of parsedObjects) {
      if (obj.typeNum === 3) {
        const { hashHex } = await gitPack.hashGitObject('blob', obj.data);
        state.blobs.set(hashHex, obj.data);
      }
    }
    for (const obj of parsedObjects) {
      if (obj.typeNum === 2) {
        const { hashHex } = await gitPack.hashGitObject('tree', obj.data);
        const entries = new Map();
        let p = 0;
        while (p < obj.data.length) {
          const nul = obj.data.indexOf(0, p);
          const header = new TextDecoder().decode(obj.data.subarray(p, nul));
          const space = header.indexOf(' ');
          const mode = header.slice(0, space);
          const name = header.slice(space + 1);
          const hashBytes = obj.data.subarray(nul + 1, nul + 21);
          const hash = gitPack.bytesToHex(hashBytes);
          p = nul + 21;
          entries.set(name, {
            mode,
            name,
            hash,
            type: mode === '40000' ? 'tree' : 'blob',
          });
        }
        state.trees.set(hashHex, entries);
      }
    }
    for (const obj of parsedObjects) {
      if (obj.typeNum === 1) {
        const { hashHex } = await gitPack.hashGitObject('commit', obj.data);
        const text = new TextDecoder().decode(obj.data);
        const [hdr, ...msgParts] = text.split('\n\n');
        const treeLine = hdr.split('\n').find((l) => l.startsWith('tree '));
        const treeHash = treeLine.slice(5).trim();
        const parents = hdr
          .split('\n')
          .filter((l) => l.startsWith('parent '))
          .map((l) => l.slice(7).trim());
        state.commits.push({
          hash: hashHex,
          treeHash,
          message: msgParts.join('\n\n').trim(),
          author: { name: 'owner-sub', email: 'docs@sorane.dev' },
          committer: { name: 'owner-sub', email: 'docs@sorane.dev' },
          parents,
          authoredAt: now(),
          committedAt: now(),
        });
        // Materialize flat file map from root tree
        const files = new Map();
        const walk = (th, prefix) => {
          const map = state.trees.get(th);
          if (!map) return;
          for (const entry of map.values()) {
            const full = prefix ? `${prefix}/${entry.name}` : entry.name;
            if (entry.type === 'tree') walk(entry.hash, full);
            else if (state.blobs.has(entry.hash)) files.set(full, state.blobs.get(entry.hash));
          }
        };
        walk(treeHash, '');
        state.files = files;
      }
    }
    state.lastPushAt = new Date().toISOString();
    return new Response('000eunpack ok\n0000', { status: 200 });
  };
  return () => {
    globalThis.fetch = origFetch;
  };
}

const req = (path, opts = {}) =>
  new Request(`${host}${path}`, {
    ...opts,
    headers: {
      origin: host,
      cookie: [
        opts.token ? `__Host-help-session=${opts.token}` : null,
        `__Host-help-browser=${browser}`,
      ]
        .filter(Boolean)
        .join('; '),
      ...(opts.headers ?? {}),
    },
  });

test('git-pack encodes sorted trees, commits, and valid PACKv2 payloads', async () => {
  const blobData = new TextEncoder().encode('# Hello\n');
  const { hashHex: blobHash } = await gitPack.hashGitObject('blob', blobData);
  const treeBytes = gitPack.encodeTree([
    { mode: '100644', name: 'z.md', hashHex: blobHash, type: 'blob' },
    { mode: '40000', name: 'a', hashHex: blobHash, type: 'tree' },
  ]);
  const { hashHex: treeHash } = await gitPack.hashGitObject('tree', treeBytes);
  assert.equal(treeHash.length, 40);
  const pack = await gitPack.buildPackfile([
    { type: 'blob', data: blobData },
    { type: 'tree', data: treeBytes },
  ]);
  assert.equal(new TextDecoder().decode(pack.subarray(0, 4)), 'PACK');
  const view = new DataView(pack.buffer, pack.byteOffset, pack.byteLength);
  assert.equal(view.getUint32(4, false), 2);
  assert.equal(view.getUint32(8, false), 2);
});

test('repos API lifecycle: create, read/write with If-Match, nested paths, tokens, fork, isolation, delete', async () => {
  const { env, artifactsRepos } = createMockEnv();
  const restoreFetch = installGitFetchMock(artifactsRepos);
  try {
    // 1. Unauthenticated API call returns 401
    const unauth = await worker.fetch(req('/api/repos'), env);
    assert.equal(unauth.status, 401);

    // 2. Create repository with README.md
    const createdRes = await worker.fetch(
      req('/api/repos', {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({
          slug: 'tech-spec',
          title: '技術仕様書 2026',
          description: 'Artifacts テスト',
          seedReadme: true,
        }),
      }),
      env,
    );
    assert.equal(createdRes.status, 201);
    const created = await createdRes.json();
    assert.equal(created.slug, 'tech-spec');
    assert.ok(created.initial_commit);

    // 3. Render GitHub-like /repos/:id UI with "Docs で開く" editor link
    const pageRes = await worker.fetch(req(`/repos/${created.id}`, { token: 'owner-token' }), env);
    assert.equal(pageRes.status, 200);
    const html = await pageRes.text();
    assert.ok(html.includes('README.md'));
    assert.ok(html.includes('/editor.html?open='));
    assert.ok(html.includes('Docs で開く'));

    // 4. Read README.md & check ETag
    const readmeRes = await worker.fetch(
      req(`/api/repos/${created.id}/blob/README.md`, { token: 'owner-token' }),
      env,
    );
    assert.equal(readmeRes.status, 200);
    const etag1 = readmeRes.headers.get('etag');
    assert.ok(etag1);
    const readmeText = await readmeRes.text();
    assert.ok(readmeText.includes('# 技術仕様書 2026'));

    // 5. Commit nested file docs/chapter1.md
    const nestedRes = await worker.fetch(
      req(`/api/repos/${created.id}/blob/docs/chapter1.md`, {
        method: 'PUT',
        token: 'owner-token',
        headers: {
          'x-csrf-token': csrf,
          'x-commit-message': encodeURIComponent('Add chapter 1'),
        },
        body: '# Chapter 1\n\nContent\n',
      }),
      env,
    );
    assert.equal(nestedRes.status, 200);
    const nested = await nestedRes.json();
    assert.ok(nested.commit);

    // 6. Stale If-Match returns 412; fresh If-Match succeeds
    const staleRes = await worker.fetch(
      req(`/api/repos/${created.id}/blob/README.md`, {
        method: 'PUT',
        token: 'owner-token',
        headers: { 'x-csrf-token': csrf, 'If-Match': etag1 },
        body: '# Updated stale',
      }),
      env,
    );
    assert.equal(staleRes.status, 412);

    const freshRes = await worker.fetch(
      req(`/api/repos/${created.id}/blob/README.md`, {
        method: 'PUT',
        token: 'owner-token',
        headers: { 'x-csrf-token': csrf, 'If-Match': nested.etag },
        body: '# Updated cleanly',
      }),
      env,
    );
    assert.equal(freshRes.status, 200);

    // 7. Other user cannot access owner's repo
    const otherRes = await worker.fetch(
      req(`/api/repos/${created.id}`, { token: 'other-token' }),
      env,
    );
    assert.equal(otherRes.status, 404);

    // 8. Mint & revoke GitOps token
    const tokRes = await worker.fetch(
      req(`/api/repos/${created.id}/tokens`, {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({ scope: 'write', ttl: 3600 }),
      }),
      env,
    );
    assert.equal(tokRes.status, 201);
    const tok = await tokRes.json();
    assert.ok(tok.plaintext);
    assert.ok(tok.remote.includes('https://'));

    const revRes = await worker.fetch(
      req(`/api/repos/${created.id}/tokens/${tok.id}`, {
        method: 'DELETE',
        token: 'owner-token',
        headers: { 'x-csrf-token': csrf },
      }),
      env,
    );
    assert.equal(revRes.status, 200);

    // 9. Fork & Delete
    const forkRes = await worker.fetch(
      req(`/api/repos/${created.id}/fork`, {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({ slug: 'tech-spec-v2' }),
      }),
      env,
    );
    assert.equal(forkRes.status, 201);
    const forked = await forkRes.json();
    assert.equal(forked.slug, 'tech-spec-v2');

    const delRes = await worker.fetch(
      req(`/api/repos/${created.id}`, {
        method: 'DELETE',
        token: 'owner-token',
        headers: { 'x-csrf-token': csrf },
      }),
      env,
    );
    assert.equal(delRes.status, 204);
  } finally {
    restoreFetch();
  }
});

test.after(async () => {
  await rm(dir, { recursive: true, force: true });
});
