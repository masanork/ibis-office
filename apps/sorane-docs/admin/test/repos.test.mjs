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
