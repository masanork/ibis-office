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
  platform: 'browser',
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
await build({
  entryPoints: ['src/diff.ts'],
  bundle: true,
  platform: 'node',
  format: 'esm',
  outfile: join(dir, 'diff.mjs'),
});
const { default: worker } = await import(pathToFileURL(join(dir, 'worker.mjs')));
const gitPack = await import(pathToFileURL(join(dir, 'git-pack.mjs')));
const diffMod = await import(pathToFileURL(join(dir, 'diff.mjs')));

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
      commitMap: new Map(), // commitHash -> commit
      refs: new Map(), // branch -> commitHash
      trees: new Map(), // treeHash -> Map(name -> entry)
      blobs: new Map(), // blobHash -> Uint8Array
      files: new Map(), // path -> Uint8Array
      tokens: new Map(),
    };
  }

  function resolveCommitFromState(state, ref) {
    if (!ref) {
      const sha = state.refs.get(state.defaultBranch);
      return sha ? state.commitMap.get(sha) ?? null : state.commits[state.commits.length - 1] ?? null;
    }
    if (state.commitMap.has(ref)) return state.commitMap.get(ref);
    const sha = state.refs.get(ref);
    return sha ? state.commitMap.get(sha) ?? null : null;
  }

  function readFileFromTree(state, treeHash, path) {
    const parts = path.split('/').filter(Boolean);
    let cur = treeHash;
    for (let i = 0; i < parts.length; i++) {
      const entries = state.trees.get(cur);
      if (!entries) return null;
      const entry = entries.get(parts[i]);
      if (!entry) return null;
      if (i === parts.length - 1) {
        return entry.type === 'blob' ? state.blobs.get(entry.hash) ?? null : null;
      }
      if (entry.type !== 'tree') return null;
      cur = entry.hash;
    }
    return null;
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
        const head = resolveCommitFromState(state, opts.ref);
        if (!head) return [];
        const out = [];
        let cur = head;
        while (cur && out.length < limit) {
          out.push(cur);
          cur = cur.parents[0] ? state.commitMap.get(cur.parents[0]) ?? null : null;
        }
        return out;
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
        return state.commitMap.get(hash) ?? null;
      },
      async readFile({ ref, path }) {
        const commit = resolveCommitFromState(state, ref);
        if (commit) {
          const b = readFileFromTree(state, commit.treeHash, path);
          return b ? new Blob([b]) : null;
        }
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
    async import({ source, target }) {
      if (artifactsRepos.has(target.name)) throw new Error('ALREADY_EXISTS');
      const state = makeRepoState(target.name, {
        description: target.opts?.description ?? null,
        setDefaultBranch: source.branch ?? 'main',
        source: source.url,
      });
      artifactsRepos.set(target.name, state);
      const repoInfo = await makeHandle(state).info();
      return {
        ...repoInfo,
        remote: state.remote,
        token: 'imp-secret',
      };
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

// Intercept fetch to simulate Artifacts and external Git Smart HTTP remotes
function installGitFetchMock(artifactsRepos, externalRemotes = new Map()) {
  const origFetch = globalThis.fetch;
  globalThis.fetch = async (input, init) => {
    const urlStr = typeof input === 'string' ? input : input.url;
    const extRefsMatch = /^(https:\/\/github\.com\/[^/]+\/[^/]+?)(?:\.git)?\/info\/refs\?service=git-upload-pack$/.exec(urlStr);
    if (extRefsMatch) {
      const ext = externalRemotes.get(extRefsMatch[1]) ?? externalRemotes.get(`${extRefsMatch[1]}.git`);
      if (!ext) return new Response('Not found', { status: 404 });
      const parts = [gitPack.encodePktLine('# service=git-upload-pack\n'), new TextEncoder().encode('0000')];
      let first = true;
      for (const [branch, sha] of ext.refs) {
        const line = first ? `${sha} refs/heads/${branch}\0multi_ack\n` : `${sha} refs/heads/${branch}\n`;
        parts.push(gitPack.encodePktLine(line));
        first = false;
      }
      parts.push(new TextEncoder().encode('0000'));
      return new Response(gitPack.concatBytes(parts), { status: 200 });
    }
    const extReceiveMatch = /^(https:\/\/github\.com\/[^/]+\/[^/]+?)(?:\.git)?\/git-receive-pack$/.exec(urlStr);
    if (extReceiveMatch) {
      const ext = externalRemotes.get(extReceiveMatch[1]) ?? externalRemotes.get(`${extReceiveMatch[1]}.git`);
      if (!ext) return new Response('Not found', { status: 404 });
      const body = new Uint8Array(await new Response(init.body).arrayBuffer());
      const headerText = new TextDecoder().decode(body.subarray(0, Math.min(body.length, 512)));
      const refLineMatch = /([0-9a-f]{40}) ([0-9a-f]{40}) refs\/heads\/([^\0\n]+)/.exec(headerText);
      const packOffset = new TextDecoder().decode(body).indexOf('PACK');
      assert.ok(packOffset >= 0, 'external git-receive-pack must contain PACK');
      const pack = body.subarray(packOffset);
      const view = new DataView(pack.buffer, pack.byteOffset, pack.byteLength);
      ext.lastPush = {
        auth: init.headers?.Authorization ?? '',
        oldSha: refLineMatch?.[1],
        newSha: refLineMatch?.[2],
        branch: refLineMatch?.[3],
        objectCount: view.getUint32(8, false),
      };
      if (refLineMatch) ext.refs.set(refLineMatch[3], refLineMatch[2]);
      return new Response('000eunpack ok\n0000', { status: 200 });
    }

    const refsMatch = /^https:\/\/artifacts\.example\/docs\/([^/]+)\.git\/info\/refs\?service=git-upload-pack$/.exec(urlStr);
    if (refsMatch) {
      const state = artifactsRepos.get(refsMatch[1]);
      if (!state) return new Response('Not found', { status: 404 });
      const parts = [gitPack.encodePktLine('# service=git-upload-pack\n'), new TextEncoder().encode('0000')];
      let first = true;
      for (const [branch, sha] of state.refs) {
        const line = first
          ? `${sha} refs/heads/${branch}\0multi_ack\n`
          : `${sha} refs/heads/${branch}\n`;
        parts.push(gitPack.encodePktLine(line));
        first = false;
      }
      parts.push(new TextEncoder().encode('0000'));
      return new Response(gitPack.concatBytes(parts), { status: 200 });
    }

    const match = /^https:\/\/artifacts\.example\/docs\/([^/]+)\.git\/git-receive-pack$/.exec(urlStr);
    if (!match) return origFetch(input, init);

    const state = artifactsRepos.get(match[1]);
    if (!state) return new Response('Not found', { status: 404 });

    const body = new Uint8Array(await new Response(init.body).arrayBuffer());
    const headerText = new TextDecoder().decode(body.subarray(0, Math.min(body.length, 512)));
    const refLineMatch = /([0-9a-f]{40}) ([0-9a-f]{40}) refs\/heads\/([^\0\n]+)/.exec(headerText);
    const packOffset = new TextDecoder().decode(body).indexOf('PACK');
    assert.ok(packOffset >= 0, 'git-receive-pack payload must contain PACK');
    const pack = body.subarray(packOffset);
    const view = new DataView(pack.buffer, pack.byteOffset, pack.byteLength);
    assert.equal(view.getUint32(4, false), 2, 'PACK version 2');
    const objCount = view.getUint32(8, false);

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
        const commitObj = {
          hash: hashHex,
          treeHash,
          message: msgParts.join('\n\n').trim(),
          author: { name: 'owner-sub', email: 'docs@sorane.dev' },
          committer: { name: 'owner-sub', email: 'docs@sorane.dev' },
          parents,
          authoredAt: now(),
          committedAt: now(),
        };
        state.commits.push(commitObj);
        state.commitMap.set(hashHex, commitObj);
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
    if (refLineMatch) {
      state.refs.set(refLineMatch[3], refLineMatch[2]);
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

    const readSource = `/repos/${created.id}?source=README.md&ref=${created.initial_commit}`;
    const sourceRes = await worker.fetch(req(readSource, { token: 'owner-token' }), env);
    assert.equal(sourceRes.status, 200);
    assert.match(await sourceRes.text(), /id="L1".*---/);
    const anonymousSource = await worker.fetch(req(readSource), env);
    assert.match(await anonymousSource.text(), /ログインが必要です/);

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

test('diff module and git-pack branch ref helpers work accurately', () => {
  const diff = diffMod.computeLineDiff(
    '# Title\n\nFirst line\nSecond line\nThird line\n',
    '# Title\n\nFirst line\nUpdated second line\nExtra line\nThird line\n',
  );
  assert.equal(diff.additions, 2);
  assert.equal(diff.deletions, 1);
  assert.equal(diff.truncated, false);
  assert.equal(diff.hunks.length, 1);
  assert.ok(diff.hunks[0].lines.some((l) => l.type === 'del' && l.text === 'Second line'));
  assert.ok(diff.hunks[0].lines.some((l) => l.type === 'add' && l.text === 'Updated second line'));

  const sha1 = 'a'.repeat(40);
  const sha2 = 'b'.repeat(40);
  const pkt = gitPack.concatBytes([
    gitPack.encodePktLine('# service=git-upload-pack\n'),
    new TextEncoder().encode('0000'),
    gitPack.encodePktLine(`${sha1} refs/heads/main\0multi_ack\n`),
    gitPack.encodePktLine(`${sha2} refs/heads/feat/md-diff\n`),
    new TextEncoder().encode('0000'),
  ]);
  const refs = gitPack.parseUploadPackRefs(pkt);
  assert.deepEqual(refs, [
    { name: 'main', sha: sha1 },
    { name: 'feat/md-diff', sha: sha2 },
  ]);
  assert.equal(gitPack.isValidBranchName('feat/review-1'), true);
  assert.equal(gitPack.isValidBranchName('../bad'), false);
  assert.equal(gitPack.isValidBranchName('bad.lock'), false);
});

test('repos branches and diff API support branch creation, cross-branch commits, and Markdown diffs', async () => {
  const { env, artifactsRepos } = createMockEnv();
  const restoreFetch = installGitFetchMock(artifactsRepos);
  try {
    const createRes = await worker.fetch(
      req('/api/repos', {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({ slug: 'md-diff-repo', title: 'Diff Test' }),
      }),
      env,
    );
    assert.equal(createRes.status, 201);
    const created = await createRes.json();

    // List branches
    const branchesRes1 = await worker.fetch(
      req(`/api/repos/${created.id}/branches`, { token: 'owner-token' }),
      env,
    );
    assert.equal(branchesRes1.status, 200);
    const b1 = await branchesRes1.json();
    assert.equal(b1.defaultBranch, 'main');
    assert.equal(b1.branches.length, 1);
    assert.equal(b1.branches[0].name, 'main');

    // Create feature branch
    const createBranchRes = await worker.fetch(
      req(`/api/repos/${created.id}/branches`, {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({ name: 'draft/review-1', fromRef: 'main' }),
      }),
      env,
    );
    assert.equal(createBranchRes.status, 201);

    // Duplicate branch fails with 409
    const dupBranchRes = await worker.fetch(
      req(`/api/repos/${created.id}/branches`, {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({ name: 'draft/review-1', fromRef: 'main' }),
      }),
      env,
    );
    assert.equal(dupBranchRes.status, 409);

    // Commit Markdown edit to draft/review-1 without touching main
    const putBranchRes = await worker.fetch(
      req(`/api/repos/${created.id}/blob/README.md`, {
        method: 'PUT',
        token: 'owner-token',
        headers: {
          'x-csrf-token': csrf,
          'x-target-branch': 'draft/review-1',
          'x-commit-message': encodeURIComponent('Update README on draft branch'),
        },
        body: '# Diff Test\n\nEdited on draft branch.\n',
      }),
      env,
    );
    assert.equal(putBranchRes.status, 200);
    const putBranch = await putBranchRes.json();
    assert.equal(putBranch.branch, 'draft/review-1');

    // Diff main vs draft/review-1
    const diffRes = await worker.fetch(
      req(`/api/repos/${created.id}/diff?base=main&head=draft/review-1`, { token: 'owner-token' }),
      env,
    );
    assert.equal(diffRes.status, 200);
    const diff = await diffRes.json();
    assert.equal(diff.files.length, 1);
    assert.equal(diff.files[0].path, 'README.md');
    assert.equal(diff.files[0].status, 'modified');
    assert.equal(diff.files[0].isText, true);
    assert.ok(diff.files[0].additions >= 1);
    assert.ok(diff.files[0].deletions >= 1);
    assert.ok(
      diff.files[0].hunks[0].lines.some(
        (l) => l.type === 'add' && l.text === 'Edited on draft branch.',
      ),
    );

    // Parent auto-resolution (head only) works
    const autoDiffRes = await worker.fetch(
      req(`/api/repos/${created.id}/diff?head=${putBranch.commit}`, { token: 'owner-token' }),
      env,
    );
    assert.equal(autoDiffRes.status, 200);
    const autoDiff = await autoDiffRes.json();
    assert.equal(autoDiff.files.length, 1);

    // Repo detail HTML renders branch selector & diff viewer
    const pageRes = await worker.fetch(
      req(`/repos/${created.id}?ref=draft/review-1`, { token: 'owner-token' }),
      env,
    );
    assert.equal(pageRes.status, 200);
    const html = await pageRes.text();
    assert.ok(html.includes('id="branch-select"'));
    assert.ok(html.includes('id="diff-card"'));
    assert.ok(html.includes('draft/review-1'));

    const state = artifactsRepos.values().next().value;
    let parent = state.commitMap.get(putBranch.commit);
    for (let index = 0; index < 30; index++) {
      const hash = createHash('sha1').update('unrelated-' + index).digest('hex');
      parent = { ...parent, hash, parents: [parent.hash], message: 'Unrelated commit ' + index };
      state.commitMap.set(hash, parent);
    }
    state.refs.set('draft/review-1', parent.hash);
    const historyUrl = `/repos/${created.id}?ref=draft/review-1&file=README.md`;
    const firstPage = await worker.fetch(req(historyUrl, { token: 'owner-token' }), env);
    const firstHistory = (await firstPage.text()).split('id="commit-history"')[1];
    assert.match(firstHistory, /さらに古い履歴/);
    assert.match(firstHistory, new RegExp('before=' + putBranch.commit));
    assert.doesNotMatch(firstHistory.split('<script>')[0], /Unrelated commit/);
    const older = await worker.fetch(
      req(historyUrl + '&before=' + putBranch.commit, { token: 'owner-token' }), env,
    );
    assert.equal(older.status, 200);
    const olderHistory = (await older.text()).split('id="commit-history"')[1];
    assert.match(olderHistory, /Update README on draft branch/);
    assert.ok(olderHistory.includes(`data-set-base="${putBranch.commit}"`));
    assert.ok(olderHistory.includes(`ref=${putBranch.commit}&amp;plain=1`));
    const immutable = await worker.fetch(
      req(`/api/repos/${created.id}/blob/README.md?ref=${putBranch.commit}&plain=1`, { token: 'owner-token' }), env,
    );
    assert.match(immutable.headers.get('Content-Type'), /^text\/plain/);
    assert.match(await immutable.text(), /Edited on draft branch/);
    const filtered = await worker.fetch(
      req(`/api/repos/${created.id}/diff?base=main&head=${putBranch.commit}&path=README.md`, { token: 'owner-token' }), env,
    );
    assert.equal(filtered.status, 200);
    assert.deepEqual((await filtered.json()).files, diff.files);

    // Other user cannot access diff or branches
    const otherDiff = await worker.fetch(
      req(`/api/repos/${created.id}/diff?base=main&head=draft/review-1`, { token: 'other-token' }),
      env,
    );
    assert.equal(otherDiff.status, 404);
  } finally {
    restoreFetch();
  }
});

test('external Git import, URL validation, and push to GitHub remote work cleanly', async () => {
  assert.equal(
    gitPack.validateExternalGitUrl('https://github.com/masanork/ibis-office.git/'),
    'https://github.com/masanork/ibis-office.git',
  );
  assert.equal(gitPack.validateExternalGitUrl('http://github.com/a/b.git'), null);
  assert.equal(gitPack.validateExternalGitUrl('https://user:pass@github.com/a/b.git'), null);
  assert.equal(gitPack.validateExternalGitUrl('https://127.0.0.1/a/b.git'), null);
  assert.equal(gitPack.validateExternalGitUrl('https://localhost/a/b.git'), null);

  const { env, artifactsRepos } = createMockEnv();
  const externalRemotes = new Map([
    ['https://github.com/owner/docs-repo.git', { refs: new Map(), lastPush: null }],
  ]);
  const restoreFetch = installGitFetchMock(artifactsRepos, externalRemotes);
  try {
    // 1. Import repository from external URL
    const impRes = await worker.fetch(
      req('/api/repos/import', {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({
          sourceUrl: 'https://github.com/owner/docs-repo.git',
          slug: 'imported-docs',
          branch: 'main',
          depth: 5,
        }),
      }),
      env,
    );
    assert.equal(impRes.status, 201);
    const imported = await impRes.json();
    assert.equal(imported.slug, 'imported-docs');

    // Reject SSRF URLs on import
    const badImp = await worker.fetch(
      req('/api/repos/import', {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({ sourceUrl: 'https://127.0.0.1/internal.git' }),
      }),
      env,
    );
    assert.equal(badImp.status, 400);

    // 2. Commit a Markdown file in Docs and push to external remote
    const putRes = await worker.fetch(
      req(`/api/repos/${imported.id}/blob/docs/guide.md`, {
        method: 'PUT',
        token: 'owner-token',
        headers: {
          'x-csrf-token': csrf,
          'x-commit-message': encodeURIComponent('Add guide.md'),
        },
        body: '# Guide\n\nHello GitHub.\n',
      }),
      env,
    );
    assert.equal(putRes.status, 200);
    const putData = await putRes.json();

    const pushRes = await worker.fetch(
      req(`/api/repos/${imported.id}/push`, {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({
          remoteUrl: 'https://github.com/owner/docs-repo.git',
          branch: 'main',
          remoteBranch: 'docs/update-guide',
          token: 'ghp_test_token',
        }),
      }),
      env,
    );
    assert.equal(pushRes.status, 200);
    const pushData = await pushRes.json();
    assert.equal(pushData.status, 'pushed');
    assert.equal(pushData.commit, putData.commit);
    assert.equal(
      pushData.compareUrl,
      'https://github.com/owner/docs-repo/compare/docs%2Fupdate-guide?expand=1',
    );

    const extState = externalRemotes.get('https://github.com/owner/docs-repo.git');
    assert.equal(extState.lastPush.branch, 'docs/update-guide');
    assert.equal(extState.lastPush.newSha, putData.commit);
    assert.ok(extState.lastPush.objectCount >= 3);

    // 3. Pushing again reports up_to_date
    const pushAgain = await worker.fetch(
      req(`/api/repos/${imported.id}/push`, {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({
          remoteUrl: 'https://github.com/owner/docs-repo.git',
          branch: 'main',
          remoteBranch: 'docs/update-guide',
          token: 'ghp_test_token',
        }),
      }),
      env,
    );
    assert.equal(pushAgain.status, 200);
    assert.equal((await pushAgain.json()).status, 'up_to_date');

    // 4. Diverged remote branch returns 409 non_fast_forward
    extState.refs.set('docs/update-guide', 'f'.repeat(40));
    const divergedPush = await worker.fetch(
      req(`/api/repos/${imported.id}/push`, {
        method: 'POST',
        token: 'owner-token',
        headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
        body: JSON.stringify({
          remoteUrl: 'https://github.com/owner/docs-repo.git',
          branch: 'main',
          remoteBranch: 'docs/update-guide',
          token: 'ghp_test_token',
        }),
      }),
      env,
    );
    assert.equal(divergedPush.status, 409);

    // 5. UI pages render import & push forms
    const homeHtml = await (await worker.fetch(req('/repos', { token: 'owner-token' }), env)).text();
    assert.ok(homeHtml.includes('id="import-repo"'));
    const detailHtml = await (
      await worker.fetch(req(`/repos/${imported.id}`, { token: 'owner-token' }), env)
    ).text();
    assert.ok(detailHtml.includes('id="push-external"'));
  } finally {
    restoreFetch();
  }
});

test.after(async () => {
  await rm(dir, { recursive: true, force: true });
});

test('public GitHub remotes share the history and comparison UI without a Docs session', async () => {
  const { env } = createMockEnv();
  const newer = 'a'.repeat(40);
  const older = 'b'.repeat(40);
  const newTree = 'c'.repeat(40);
  const oldTree = 'd'.repeat(40);
  const newBlob = 'e'.repeat(40);
  const oldBlob = 'f'.repeat(40);
  const makeCommit = (sha, tree, message, parents) => ({
    sha, parents: parents.map(sha => ({ sha })),
    commit: {
      message, tree: { sha: tree },
      author: { name: 'Author', date: '2026-10-01T00:00:00Z' },
      committer: { name: 'Author', date: '2026-10-01T00:00:00Z' },
    },
  });
  const head = makeCommit(newer, newTree, '改正', [older]);
  const base = makeCommit(older, oldTree, '旧版', []);
  const original = globalThis.fetch;
  globalThis.fetch = async (input, init) => {
    const url = new URL(input);
    assert.equal(init.redirect, 'manual');
    assert.equal(new Headers(init.headers).has('Authorization'), false);
    if (url.origin === 'https://raw.githubusercontent.com') {
      assert.equal(url.pathname, '/owner/law/' + older + '/law.md');
      return new Response('旧条文\n');
    }
    assert.equal(url.origin, 'https://api.github.com');
    assert.ok(url.pathname.startsWith('/repos/owner/law'));
    switch (url.pathname.slice('/repos/owner/law'.length)) {
      case '': return Response.json({ private: false, default_branch: 'main' });
      case '/branches': return Response.json([{ name: 'main', commit: { sha: newer } }, { name: 'upcoming', commit: { sha: newer } }]);
      case '/commits':
        assert.equal(url.searchParams.get('path'), 'law.md');
        assert.equal(url.searchParams.get('sha'), newer);
        return Response.json([head, base], { headers: { Link: '<https://api.github.com/example>; rel="next"' } });
      case '/commits/main':
      case '/commits/' + newer: return Response.json(head);
      case '/commits/' + older: return Response.json(base);
      case '/git/trees/' + newTree: return Response.json({ tree: [{ path: 'law.md', mode: '100644', sha: newBlob, type: 'blob' }] });
      case '/git/trees/' + oldTree: return Response.json({ tree: [{ path: 'law.md', mode: '100644', sha: oldBlob, type: 'blob' }] });
      case '/git/blobs/' + newBlob: return new Response('新条文\n');
      case '/git/blobs/' + oldBlob: return new Response('旧条文\n');
      default: assert.fail(url.href);
    }
  };
  try {
    const open = await worker.fetch(req('/repos/open?url=https%3A%2F%2Fgithub.com%2Fowner%2Flaw.git'), env);
    assert.equal(open.status, 303);
    assert.equal(new URL(open.headers.get('Location')).pathname, '/repos/github/owner/law');
    const bad = await worker.fetch(req('/repos/open?url=https%3A%2F%2Fexample.com%2Fowner%2Flaw'), env);
    assert.equal(bad.status, 400);
    const page = await worker.fetch(req('/repos/github/owner/law?file=law.md'), env);
    assert.equal(page.status, 200);
    const html = await page.text();
    assert.match(html, /読み取り専用/);
    assert.match(html, /upcoming/);
    assert.ok(html.includes('before=2&amp;anchor=' + newer));
    assert.ok(html.includes('data-set-base="' + older + '"'));
    assert.doesNotMatch(html, /id="(commit-file|mint-write|push-external)"/);
    const path = '/api/repos/github/owner/law';
    const blob = await worker.fetch(req(path + '/blob/law.md?ref=' + older), env);
    assert.equal(blob.status, 200);
    assert.equal(await blob.text(), '旧条文\n');
    const diff = await worker.fetch(req(path + '/diff?path=law.md&base=' + older + '&head=' + newer), env);
    assert.equal(diff.status, 200);
    const files = (await diff.json()).files;
    assert.equal(files.length, 1);
    assert.equal(files[0].additions, 1);
    assert.equal(files[0].deletions, 1);
    assert.ok(files[0].hunks[0].lines.some(line => line.text === '新条文'));
  } finally {
    globalThis.fetch = original;
  }
});

test('repository source follows Markdown links at the same commit and escapes source', async () => {
  const { env } = createMockEnv();
  const commit = '1'.repeat(40);
  const root = '2'.repeat(40);
  const docs = '3'.repeat(40);
  const api = '4'.repeat(40);
  const md = '5'.repeat(40);
  const yaml = '6'.repeat(40);
  const json = '7'.repeat(40);
  const badYaml = '8'.repeat(40);
  const badJson = '9'.repeat(40);
  const openapi = 'a'.repeat(40);
  const openapi30 = 'b'.repeat(40);
  const openapi31 = 'c'.repeat(40);
  const payloadSchema = 'd'.repeat(40);
  const postalSchema = 'e'.repeat(40);
  let payloadReads = 0;
  const feature = 'f'.repeat(40);
  const badFeature = '0'.repeat(40);
  const original = globalThis.fetch;
  globalThis.fetch = async (input) => {
    const url = new URL(input);
    assert.equal(url.origin, 'https://api.github.com');
    const path = url.pathname.replace(/^\/repos\/owner\/spec/, '');
    switch (path) {
      case '': return Response.json({ private: false, default_branch: 'main' });
      case '/commits/main':
      case '/commits/' + commit:
        return Response.json({
          sha: commit, parents: [], commit: {
            message: 'Specification', tree: { sha: root },
            author: { name: 'Author', date: '2026-10-01T00:00:00Z' },
            committer: { name: 'Author', date: '2026-10-01T00:00:00Z' },
          },
        });
      case '/git/trees/' + root:
        return Response.json({ tree: [
          { path: 'docs', mode: '040000', sha: docs, type: 'tree' },
          { path: 'api', mode: '040000', sha: api, type: 'tree' },
        ] });
      case '/git/trees/' + docs:
        return Response.json({ tree: [
          { path: 'README.md', mode: '100644', sha: md, type: 'blob' },
          { path: 'address.feature', mode: '100644', sha: feature, type: 'blob' },
          { path: 'broken.feature', mode: '100644', sha: badFeature, type: 'blob' },
        ] });
      case '/git/trees/' + api:
        return Response.json({ tree: [
          { path: '契約.yaml', mode: '100644', sha: yaml, type: 'blob' },
          { path: 'schema.json', mode: '100644', sha: json, type: 'blob' },
          { path: 'broken.yaml', mode: '100644', sha: badYaml, type: 'blob' },
          { path: 'broken.json', mode: '100644', sha: badJson, type: 'blob' },
          { path: 'openapi.yaml', mode: '100644', sha: openapi, type: 'blob' },
          { path: 'openapi30.yaml', mode: '100644', sha: openapi30, type: 'blob' },
          { path: 'openapi31.json', mode: '100644', sha: openapi31, type: 'blob' },
          { path: 'payload.yaml', mode: '100644', sha: payloadSchema, type: 'blob' },
          { path: 'postal.json', mode: '100644', sha: postalSchema, type: 'blob' },
        ] });
      case '/git/blobs/' + md:
        return new Response([
          '# API仕様', '## REQ-42 住所変更', '## REQ-43 郵便番号', '## REQ-99 未対応',
          '## REQ-example 例テーブル', '[API](../api/%E5%A5%91%E7%B4%84.yaml#L2)', '[Schema](../api/schema.json)', '[Acceptance](address.feature#L6)', '[Broken](broken.feature)', '[external](//other.test/external.json)',
          '[escape](../../escape.json)', '[slash](../api%2fother.json)', '```md', '## REQ-fake', '[example](sample.json)', '```',
        ].join('\n'));
      case '/git/blobs/' + yaml:
        return new Response([
          'openapi: 3.1.0', 'info: "<script>alert(1)</script>"',
          'paths: &items', '  - first', '  - *items', '---', 'another: true',
        ].join('\r\n'));
      case '/git/blobs/' + json:
        return new Response('{\n  "type":"object",\n  "limit": 9007199254740993,\n  "$defs": {\n    "a/b~c": [\n      {"title":"Linked definition"}\n    ],\n    "duplicate": 1, "duplicate": 2\n  }\n}');
      case '/git/blobs/' + badYaml:
        return new Response('paths:\n  - [unfinished');
      case '/git/blobs/' + badJson:
        return new Response('{type: object}');
      case '/git/blobs/' + openapi:
        return new Response([
          'openapi: 3.2.0', 'paths:', '  /pets:', '    get:', '      summary: List pets',
          '      responses:', '        "200":', '          $ref: "#/components/responses/OK"',
          '    query: {}', '    additionalOperations:', '      COPY: {}',
          'components:', '  responses:', '    OK:', '      description: Success',
          '  schemas:', '    Pet:', '      $ref: "schema.json#/%24defs/a~1b~0c/0/title"',
          '    Foreign:', '      $ref: "https://example.test/spec.json#/a"',
          '    Rebased:', '      $id: "https://example.test/schema.json"',
          '      $ref: "schema.json#/$defs/duplicate"',
        ].join('\n'));
      case '/git/blobs/' + openapi30:
        return new Response([
          'openapi: 3.0.4', 'info: {title: API, version: "1"}', 'paths:',
          '  /first:', '    get:', '      operationId: same',
          '  /second:', '    post:', '      operationId: same', '      responses: {}',
          '  missing-slash: {}', '  /not-an-object: true',
        ].join('\n'));
      case '/git/blobs/' + payloadSchema:
        payloadReads++;
        return new Response([
          '$defs:', '  Payload:', '    type: object', '    properties:',
          '      postalCode:', '        $ref: ./postal.json',
        ].join('\n'));
      case '/git/blobs/' + postalSchema:
        return Response.json({ $schema: 'https://json-schema.org/draft/2020-12/schema', type: 'string' });
      case '/git/blobs/' + openapi31:
        return new Response(JSON.stringify({
          openapi: '3.1.1',
          info: { title: 'API', version: '1' },
          components: { schemas: {
            Address: {
              type: 'object', required: ['postalCode'],
              properties: { postalCode: { $ref: '#/components/schemas/PostalCode' } },
            },
            PostalCode: { type: 'string' },
            Recursive: { $ref: '#/components/schemas/Recursive' },
          } },
          paths: {
            '/address': {
              post: {
                requestBody: { content: { 'application/json': {
                  schema: { $ref: '#/components/schemas/Address', description: 'Mailing address' },
                  examples: {
                    good: { value: { postalCode: '1000001' } },
                    invalid: { value: { postalCode: 1000001 } },
                    missing: { value: { city: 'Tokyo' } },
                  },
                } } },
                responses: {
                  '200': { content: { 'application/problem+json': {
                    schema: { type: 'object', properties: { numbers: { type: 'array', items: { type: 'integer' } } } },
                    example: { numbers: ['wrong'] },
                  } } },
                  '201': { content: { 'application/json': {
                    schema: { $ref: 'payload.yaml#/$defs/Payload' },
                    examples: {
                      good: { value: { postalCode: '1000001' } },
                      invalid: { value: { postalCode: 9876543 } },
                    },
                  } } },
                  '400': { content: { 'application/json': { schema: { type: 'string', pattern: '^a' }, example: 'b' } } },
                  '500': { content: { 'application/json': { schema: { $ref: './schema.json' }, example: {} } } },
                  '501': { content: { 'application/json': { schema: { $ref: '#/components/schemas/Recursive' }, example: {} } } },
                  '502': { content: { 'application/json': { schema: { $ref: '#/components/schemas/Missing' }, example: {} } } },
                  '503': { content: { 'application/json': {
                    schema: { $ref: '#/components/schemas/PostalCode', minLength: 5 }, example: 'one',
                  } } },
                },
              },
            },
          },
        }, null, 2));
      case '/git/blobs/' + feature:
        return new Response([
          '# language: ja', '@REQ-42', '機能: 住所変更', '  ルール: 郵便番号を必須にする',
          '    @REQ-43', '    シナリオアウトライン: 郵便番号を検証 <番号>',
          '      前提 住所が登録済み', '      もし <番号> を入力', '      ならば 結果を表示',
          '      例: 有効な入力', '        | 番号 |', '        | 1000001 |',
          '      @REQ-example', '      例: タグ付き', '        | 番号 |', '        | 1000002 |',
        ].join('\r\n'));
      case '/git/blobs/' + badFeature:
        return new Response('Scenario: No feature heading\n');
      default: assert.fail(url.href);
    }
  };
  try {
    const base = '/repos/github/owner/spec';
    const page = await worker.fetch(req(base + '?source=docs%2FREADME.md'), env);
    assert.equal(page.status, 200);
    const html = await page.text();
    assert.match(html, /API仕様/);
    const link = base + '?ref=' + commit + '&source=api%2F%E5%A5%91%E7%B4%84.yaml#L2';
    assert.ok(html.includes('href="' + link.replace('&', '&amp;') + '"'));
    assert.doesNotMatch(html, /href="[^"]*(external\.json|escape\.json|other\.json|sample\.json)/);
    const linked = await worker.fetch(req(link), env);
    assert.equal(linked.status, 200);
    const source = await linked.text();
    assert.match(source, /openapi: 3\.1\.0/);
    assert.match(source, /id="L2"/);
    assert.match(source, /aria-label="構造ツリー"/);
    assert.match(source, /href="#L2" aria-label="info（2 行目）"/);
    assert.match(source, /href="#L5" aria-label="\[1\]（5 行目）"/);
    assert.match(source, /\*items<\/code>/);
    assert.match(source, /ドキュメント 2/);
    assert.match(source, /href="#L7" aria-label="another（7 行目）"/);
    assert.match(source, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
    assert.match(source, new RegExp('ref=' + commit + '&amp;plain=1'));
    const schemaLink = base + '?ref=' + commit + '&source=api%2Fschema.json';
    assert.ok(html.includes('href="' + schemaLink.replace('&', '&amp;') + '"'));
    const schema = await worker.fetch(req(schemaLink), env);
    assert.equal(schema.status, 200);
    const schemaSource = await schema.text();
    assert.match(schemaSource, /&quot;type&quot;:&quot;object&quot;/);
    assert.match(schemaSource, /href="#L3" aria-label="limit（3 行目）"/);
    assert.match(schemaSource, /9007199254740993<\/code>/);
    const malformed = await worker.fetch(req(base + '?ref=' + commit + '&source=api%2Fbroken.yaml'), env);
    assert.equal(malformed.status, 200);
    const malformedSource = await malformed.text();
    assert.match(malformedSource, /構文エラー/);
    assert.match(malformedSource, /href="#L2">2 行目/);
    assert.match(malformedSource, /\[unfinished/);
    const nonJson = await worker.fetch(req(base + '?ref=' + commit + '&source=api%2Fbroken.json'), env);
    const nonJsonSource = await nonJson.text();
    assert.match(nonJsonSource, /JSONの構文を確認してください/);
    assert.doesNotMatch(nonJsonSource, /aria-label="構造ツリー"/);
    const apiPage = await worker.fetch(req(base + '?ref=' + commit + '&source=api%2Fopenapi.yaml'), env);
    assert.equal(apiPage.status, 200);
    const apiHtml = await apiPage.text();
    assert.match(apiHtml, /aria-label="OpenAPI 操作"/);
    assert.match(apiHtml, /aria-label="OpenAPI 基本チェック"/);
    assert.match(apiHtml, /info はマッピングで指定してください/);
    assert.doesNotMatch(apiHtml, /responses はマッピングで指定してください/);
    const diagnostics30 = await worker.fetch(req(base + '?ref=' + commit + '&source=api%2Fopenapi30.yaml'), env);
    const html30 = await diagnostics30.text();
    assert.match(html30, /href="#L6">6 行目<\/a> · responses はマッピングで指定してください/);
    assert.match(html30, /href="#L9">9 行目<\/a> · operationId が同じ文書の別の操作と重複/);
    assert.match(html30, /href="#L10">10 行目<\/a> · responses に応答コード/);
    assert.match(html30, /href="#L11">11 行目<\/a> · パス名/);
    assert.match(html30, /href="#L12">12 行目<\/a> · パスの定義/);
    const diagnostics31 = await worker.fetch(req(base + '?ref=' + commit + '&source=api%2Fopenapi31.json'), env);
    const html31 = await diagnostics31.text();
    assert.match(html31, /確認対象の項目に問題は見つかりませんでした/);
    const checks = html31.split('aria-label="OpenAPI JSON 例チェック"')[1].split('</aside>')[0];
    assert.match(checks, /6 件を検証し、4 件に問題/);
    assert.match(checks, /5 件は対応するスキーマ参照やキーワード/);
    const postedCodeLine = /<span id="L(\d+)">[^\n]*&quot;postalCode&quot;: 1000001/.exec(html31)[1];
    const responseValueLine = /<span id="L(\d+)">[^\n]*&quot;wrong&quot;/.exec(html31)[1];
    assert.ok(checks.includes('href="#L' + postedCodeLine + '"'));
    assert.ok(checks.includes('href="#L' + responseValueLine + '"'));
    const splitCodeLine = /<span id="L(\d+)">[^\n]*&quot;postalCode&quot;: 9876543/.exec(html31)[1];
    assert.ok(checks.includes('href="#L' + splitCodeLine + '"'));
    assert.equal(payloadReads, 1);
    assert.match(checks, /required property &quot;postalCode&quot;/);
    assert.match(apiHtml, /href="#L4"><code>GET \/pets<\/code>/);
    assert.match(apiHtml, /href="#L9"><code>QUERY \/pets<\/code>/);
    assert.match(apiHtml, /href="#L11"><code>COPY \/pets<\/code>/);
    const pointer = '/$defs/a~1b~0c/0/title';
    const externalRef = schemaLink + '&' + new URLSearchParams({ pointer }) + '#definition';
    assert.ok(apiHtml.includes('href="' + externalRef.replaceAll('&', '&amp;') + '"'));
    assert.doesNotMatch(apiHtml, /href="[^"]*(example\.test|duplicate)/);
    const definition = await worker.fetch(req(externalRef), env);
    assert.match(await definition.text(), /id="L6".*id="definition".*Linked definition/);
    const localRef = base + '?ref=' + commit + '&source=api%2Fopenapi.yaml&' +
      new URLSearchParams({ pointer: '/components/responses/OK' }) + '#definition';
    assert.ok(apiHtml.includes('href="' + localRef.replaceAll('&', '&amp;') + '"'));
    const local = await worker.fetch(req(localRef), env);
    assert.match(await local.text(), /id="L14".*id="definition".*OK:/);
    for (const pointer of ['/$defs/duplicate', '/$defs/a~1b~0c/00/title', '/missing']) {
      const response = await worker.fetch(req(schemaLink + '&' + new URLSearchParams({ pointer })), env);
      const html = await response.text();
      assert.match(html, /指定された参照先を特定できませんでした/);
      assert.doesNotMatch(html, /id="definition"/);
    }
    const featureLink = base + '?ref=' + commit + '&source=docs%2Faddress.feature#L6';
    assert.ok(html.includes('href="' + featureLink.replaceAll('&', '&amp;') + '"'));
    const trace = html.split('aria-label="要求と受け入れテスト"')[1].split('</nav>')[0];
    assert.match(trace, /href="#L2">REQ-42<\/a>/);
    assert.match(trace, /href="#L3">REQ-43<\/a>/);
    assert.equal(trace.split('href="' + featureLink.replaceAll('&', '&amp;') + '"').length - 1, 2);
    assert.match(trace, /href="#L4">REQ-99<\/a>\s*<span class="muted">確認できたリンク先に一致なし/);
    assert.match(trace, /href="#L5">REQ-example<\/a>/);
    assert.ok(trace.includes('href="' + featureLink.replaceAll('&', '&amp;').replace('#L6', '#L14') + '"'));
    assert.doesNotMatch(trace, /REQ-fake/);
    assert.match(trace, /1 件の \.feature を確認できませんでした/);
    const featureResponse = await worker.fetch(req(featureLink), env);
    assert.equal(featureResponse.status, 200);
    const featureHtml = await featureResponse.text();
    assert.match(featureHtml, /aria-label="Gherkin シナリオ"/);
    assert.match(featureHtml, /href="#L2">@REQ-42<\/a>/);
    assert.match(featureHtml, /href="#L4">ルール: 郵便番号を必須にする/);
    assert.match(featureHtml, /href="#L6">シナリオアウトライン: 郵便番号を検証 &lt;番号&gt;/);
    assert.match(featureHtml, /href="#L10">例: 有効な入力/);
    const brokenFeature = await worker.fetch(req(base + '?ref=' + commit + '&source=docs%2Fbroken.feature'), env);
    assert.equal(brokenFeature.status, 200);
    const brokenHtml = await brokenFeature.text();
    assert.match(brokenHtml, /aria-label="Gherkin 構文エラー"/);
    assert.match(brokenHtml, /href="#L1">1 行目/);
    assert.match(brokenHtml, /id="L1".*Scenario: No feature heading/);
    const invalid = await worker.fetch(req(base + '?source=..%2FREADME.md'), env);
    assert.equal(invalid.status, 400);
  } finally {
    globalThis.fetch = original;
  }
});
