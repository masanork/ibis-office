// SPDX-License-Identifier: AGPL-3.0-only
import {
  BROWSER,
  baseHeaders,
  cookie,
  cookieHeader,
  current,
  escape,
  fail,
  formCsrf,
  hash,
  random,
  type Session,
} from '../vendor/mikaki-oidc';
import {
  ConflictError,
  commitFilesToArtifactsRepo,
  normalizeRepoPath,
} from './git-pack';
import {
  USER_MAX_FILE,
  bareEtag,
  boundedBody,
  looksLikeOffice,
  quoteEtag,
} from './office-file';
import { NEXT, type FileEnv } from './files';

export type RepoEnv = FileEnv & {
  ARTIFACTS?: Artifacts;
};

type RepoRow = {
  id: string;
  issuer: string;
  sub: string;
  repo_name: string;
  slug: string;
  title: string;
  description: string;
  default_branch: string;
  created_at: number;
  updated_at: number;
};

const MAX_TEXT_FILE = 4 * 1024 * 1024;
const REPO_PAGE_PATH = /^\/repos\/([0-9a-f-]{36})$/i;
const REPO_API_ITEM = /^\/api\/repos\/([0-9a-f-]{36})$/i;
const REPO_API_BLOB = /^\/api\/repos\/([0-9a-f-]{36})\/blob\/(.+)$/i;
const REPO_API_TOKENS = /^\/api\/repos\/([0-9a-f-]{36})\/tokens$/i;
const REPO_API_TOKEN_ITEM = /^\/api\/repos\/([0-9a-f-]{36})\/tokens\/([^/]+)$/i;
const REPO_API_FORK = /^\/api\/repos\/([0-9a-f-]{36})\/fork$/i;

export function isReposApi(path: string): boolean {
  return path === '/api/repos' || path.startsWith('/api/repos/');
}

export function isReposRoute(path: string): boolean {
  return path === '/repos' || path.startsWith('/repos/') || isReposApi(path);
}

function json(env: RepoEnv, body: unknown, status = 200, browser?: string): Response {
  const headers = baseHeaders(env);
  headers.set('Content-Type', 'application/json; charset=utf-8');
  if (browser) headers.append('Set-Cookie', cookieHeader(BROWSER, browser, 86400));
  return new Response(JSON.stringify(body), { status, headers });
}

function reposPage(
  env: RepoEnv,
  title: string,
  content: string,
  nextPath = '/repos',
  status = 200,
  browser?: string,
): Response {
  const headers = baseHeaders(env);
  headers.set('Content-Type', 'text/html; charset=utf-8');
  headers.set(
    'Content-Security-Policy',
    `default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'self' ${env.ISSUER}; frame-ancestors 'none'; base-uri 'none'`,
  );
  if (browser) headers.append('Set-Cookie', cookieHeader(BROWSER, browser, 86400));
  headers.append('Set-Cookie', cookieHeader(NEXT, nextPath, 600));
  const css = `
    :root{--bg:#f6f8fa;--card:#ffffff;--border:#d0d7de;--fg:#1f2328;--muted:#656d76;--accent:#0969da;--danger:#cf222e;--ok:#1a7f37}
    *{box-sizing:border-box}
    body{font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Helvetica,Arial,sans-serif;background:var(--bg);color:var(--fg);margin:0}
    header.topbar{background:#fff;border-bottom:1px solid var(--border);padding:.75rem 1.5rem;display:flex;align-items:center;justify-content:space-between;gap:1rem}
    header.topbar nav{display:flex;gap:1rem;align-items:center;font-size:.92rem}
    a{color:var(--accent);text-decoration:none}
    a:hover{text-decoration:underline}
    main{max-width:64rem;margin:1.5rem auto;padding:0 1.25rem}
    .card{background:var(--card);border:1px solid var(--border);border-radius:8px;overflow:hidden;margin-bottom:1rem}
    .card-hd{background:#f6f8fa;border-bottom:1px solid var(--border);padding:.65rem 1rem;display:flex;align-items:center;justify-content:space-between;gap:.75rem;flex-wrap:wrap}
    .card-bd{padding:1rem}
    .row{display:flex;align-items:center;justify-content:space-between;gap:.75rem;padding:.65rem 1rem;border-top:1px solid var(--border)}
    .row:first-child{border-top:none}
    .badge{display:inline-block;padding:.1rem .45rem;border:1px solid var(--border);border-radius:999px;font-size:.75rem;color:var(--muted);background:#fff}
    .btn,button{display:inline-flex;align-items:center;gap:.35rem;font:inherit;padding:.4rem .75rem;border-radius:6px;border:1px solid var(--border);background:#f6f8fa;color:var(--fg);cursor:pointer;text-decoration:none}
    .btn:hover,button:hover{background:#eaeef2;text-decoration:none}
    .btn-primary,button.primary{background:var(--ok);border-color:var(--ok);color:#fff}
    .btn-primary:hover,button.primary:hover{background:#166c2d}
    .btn-danger,button.danger{color:var(--danger)}
    input,textarea,select{font:inherit;padding:.45rem .6rem;border:1px solid var(--border);border-radius:6px;width:100%;background:#fff}
    label{display:block;margin:.6rem 0;font-weight:600}
    label small{font-weight:400;color:var(--muted);display:block}
    .grid2{display:grid;grid-template-columns:1fr 1fr;gap:1rem}
    @media(max-width:720px){.grid2{grid-template-columns:1fr}}
    pre.code{background:#f6f8fa;border:1px solid var(--border);border-radius:6px;padding:.75rem;overflow:auto;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.82rem;margin:0}
    .muted{color:var(--muted);font-size:.85rem}
    #status[data-kind=error]{color:var(--danger)}
    #status[data-kind=ok]{color:var(--ok)}
  `;
  return new Response(
    `<!doctype html><html lang="ja"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${escape(title)} · Docs</title><style>${css}</style><body><header class="topbar"><div><strong>Docs</strong> <span class="badge">Artifacts GitHub</span></div><nav><a href="/repos">リポジトリ</a><a href="/files">保存したファイル</a><a href="/">ホーム</a></nav></header><main>${content}</main></body></html>`,
    { status, headers },
  );
}

async function ensureBrowser(request: Request): Promise<{ browser: string; fresh: boolean }> {
  const existing = cookie(request, BROWSER);
  if (existing) return { browser: existing, fresh: false };
  return { browser: random(), fresh: true };
}

async function requireUser(request: Request, env: RepoEnv, forceCheck = false): Promise<Session> {
  const session = await current(request, env, forceCheck);
  if (!session) fail(401, 'login_required');
  return session;
}

async function csrfOk(request: Request, env: RepoEnv): Promise<void> {
  if (request.headers.get('origin') !== env.RP_ORIGIN) fail(403, 'invalid_origin');
  const browser = cookie(request, BROWSER);
  if (!browser || request.headers.get('x-csrf-token') !== (await hash(browser))) {
    fail(403, 'invalid_csrf');
  }
}

function requireArtifacts(env: RepoEnv): Artifacts {
  if (!env.ARTIFACTS) fail(503, 'artifacts_not_configured');
  return env.ARTIFACTS;
}

async function ownedRepo(env: RepoEnv, session: Session, id: string): Promise<RepoRow> {
  const row = await env.DB.prepare(
    'SELECT id,issuer,sub,repo_name,slug,title,description,default_branch,created_at,updated_at FROM doc_repos WHERE id=? AND issuer=? AND sub=?',
  )
    .bind(id, env.ISSUER, session.sub)
    .first<RepoRow>();
  if (!row) fail(404, 'not_found');
  return row;
}

function sanitizeSlug(raw: string): string | null {
  const slug = raw
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '');
  if (!slug || slug.length > 48) return null;
  return slug;
}

function isEditableInIbis(path: string): boolean {
  return /\.(md|markdown|docx|xlsx|pptx)$/i.test(path);
}

function contentTypeForPath(path: string): string {
  const lower = path.toLowerCase();
  if (lower.endsWith('.md') || lower.endsWith('.markdown')) return 'text/markdown; charset=utf-8';
  if (lower.endsWith('.txt')) return 'text/plain; charset=utf-8';
  if (lower.endsWith('.json')) return 'application/json; charset=utf-8';
  if (lower.endsWith('.xml') || lower.endsWith('.rels')) return 'application/xml; charset=utf-8';
  return 'application/octet-stream';
}

function starterReadme(title: string, description: string): Uint8Array {
  const safeTitle = title.replace(/[\r\n]+/g, ' ').trim() || '無題のドキュメント';
  const safeDesc = description.replace(/[\r\n]+/g, ' ').trim();
  const content = `---
title: ${safeTitle}
size: a4
columns: 1
${safeDesc ? `description: ${safeDesc}\n` : ''}---

# ${safeTitle}

このリポジトリは **Docs (ibisOffice) + Cloudflare Artifacts** で管理されています。
ブラウザ上のエディタでの編集と、LLM / CLI からの \`git push\` の両方が同じ Git 履歴として記録されます。

## タスク

- [ ] ドキュメントの構成をレビューする
- [x] Git リポジトリを作成する
`;
  return new TextEncoder().encode(content);
}

async function readTreeAtPath(
  repo: ArtifactsRepo,
  branch: string,
  dirPath: string,
): Promise<{
  headCommit: ArtifactsCommitMetadata | null;
  entries: ArtifactsTreeEntry[];
}> {
  const commits = await repo.log({ ref: branch, limit: 1 });
  const head = commits[0] ?? null;
  if (!head) return { headCommit: null, entries: [] };

  let treeHash = head.treeHash;
  if (dirPath) {
    const segments = dirPath.split('/').filter(Boolean);
    for (const seg of segments) {
      const children = (await repo.readTree(treeHash)) ?? [];
      const found = children.find((c) => c.name === seg && c.type === 'tree');
      if (!found) fail(404, 'not_found');
      treeHash = found.hash;
    }
  }
  const entries = (await repo.readTree(treeHash)) ?? [];
  entries.sort((a, b) => {
    if (a.type === 'tree' && b.type !== 'tree') return -1;
    if (a.type !== 'tree' && b.type === 'tree') return 1;
    return a.name.localeCompare(b.name);
  });
  return { headCommit: head, entries };
}

export async function listRepos(request: Request, env: RepoEnv): Promise<Response> {
  const session = await requireUser(request, env);
  const { browser, fresh } = await ensureBrowser(request);
  const rows = await env.DB.prepare(
    'SELECT id,repo_name,slug,title,description,default_branch,created_at,updated_at FROM doc_repos WHERE issuer=? AND sub=? ORDER BY updated_at DESC LIMIT 100',
  )
    .bind(env.ISSUER, session.sub)
    .all<Omit<RepoRow, 'issuer' | 'sub'>>();
  return json(
    env,
    {
      csrf: await hash(browser),
      repos: rows.results ?? [],
    },
    200,
    fresh ? browser : undefined,
  );
}

export async function createRepo(request: Request, env: RepoEnv): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);

  const body = (await request.json().catch(() => null)) as {
    slug?: string;
    title?: string;
    description?: string;
    seedReadme?: boolean;
  } | null;
  if (!body || typeof body !== 'object') fail(400, 'invalid_metadata');
  const slug = sanitizeSlug(body.slug ?? '');
  const title = (body.title ?? '').trim();
  const description = (body.description ?? '').trim().slice(0, 200);
  if (!slug || !title || title.length > 100) fail(400, 'invalid_metadata');

  const id = crypto.randomUUID();
  const repoName = `doc-${id.slice(0, 8)}-${slug}`;
  const defaultBranch = 'main';
  const created = await artifacts.create(repoName, {
    description: description || title,
    setDefaultBranch: defaultBranch,
  });

  const now = Math.floor(Date.now() / 1000);
  await env.DB.prepare(
    'INSERT INTO doc_repos(id,issuer,sub,repo_name,slug,title,description,default_branch,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)',
  )
    .bind(id, env.ISSUER, session.sub, repoName, slug, title, description, defaultBranch, now, now)
    .run();

  let initialCommit: string | null = null;
  if (body.seedReadme !== false) {
    using repo = await artifacts.get(repoName);
    const committed = await commitFilesToArtifactsRepo({
      repo,
      branch: defaultBranch,
      message: 'Initial commit: add README.md',
      authorName: session.sub.slice(0, 16),
      authorEmail: 'docs@sorane.dev',
      changes: [{ path: 'README.md', content: starterReadme(title, description) }],
    });
    initialCommit = committed.commitHash;
  }

  return json(
    env,
    {
      id,
      slug,
      repo_name: repoName,
      title,
      description,
      default_branch: defaultBranch,
      remote: created.remote,
      initial_commit: initialCommit,
    },
    201,
  );
}

export async function getRepoDetail(request: Request, env: RepoEnv, id: string): Promise<Response> {
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  const url = new URL(request.url);
  const dirPath = normalizeRepoPath(url.searchParams.get('path') ?? '') ?? '';
  const ref = url.searchParams.get('ref')?.trim() || row.default_branch;

  using repo = await artifacts.get(row.repo_name);
  const info = await repo.info();
  const { headCommit, entries } = await readTreeAtPath(repo, ref, dirPath);
  const commits = await repo.log({ ref, limit: 20 });

  return json(env, {
    repo: {
      ...row,
      remote: info.remote,
      lastPushAt: info.lastPushAt,
    },
    ref,
    path: dirPath,
    headCommit,
    entries,
    commits,
  });
}

export async function readRepoBlob(
  request: Request,
  env: RepoEnv,
  id: string,
  rawPath: string,
): Promise<Response> {
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  const path = normalizeRepoPath(decodeURIComponent(rawPath));
  if (!path) fail(400, 'invalid_path');

  const ref = new URL(request.url).searchParams.get('ref')?.trim() || row.default_branch;
  using repo = await artifacts.get(row.repo_name);
  const commits = await repo.log({ ref, limit: 1 });
  const headHash = commits[0]?.hash ?? '';
  const blob = await repo.readFile({ ref, path });
  if (!blob) fail(404, 'not_found');

  const headers = baseHeaders(env);
  headers.set('Content-Type', contentTypeForPath(path));
  const fileName = path.split('/').pop() ?? 'document';
  headers.set('Content-Disposition', `inline; filename="${fileName}"`);
  if (headHash) headers.set('ETag', quoteEtag(headHash));

  if (request.method === 'HEAD') {
    return new Response(null, { status: 200, headers });
  }
  return new Response(blob, { status: 200, headers });
}

export async function writeRepoBlob(
  request: Request,
  env: RepoEnv,
  id: string,
  rawPath: string,
): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  const path = normalizeRepoPath(decodeURIComponent(rawPath));
  if (!path) fail(400, 'invalid_path');

  const lower = path.toLowerCase();
  const isOffice = lower.endsWith('.docx') || lower.endsWith('.xlsx') || lower.endsWith('.pptx');
  const bytes = await boundedBody(request, isOffice ? USER_MAX_FILE : MAX_TEXT_FILE);
  if (isOffice && !looksLikeOffice(bytes, lower)) fail(400, 'invalid_file');

  const ifMatch = request.headers.get('if-match');
  const expectedParentSha =
    ifMatch && ifMatch.trim() !== '*' ? bareEtag(ifMatch.trim()) : undefined;
  const rawMsg = request.headers.get('x-commit-message');
  let commitMessage = `Update ${path}`;
  if (rawMsg) {
    try {
      const decoded = decodeURIComponent(rawMsg).trim();
      if (decoded) commitMessage = decoded.slice(0, 200);
    } catch {
      commitMessage = rawMsg.trim().slice(0, 200) || commitMessage;
    }
  }

  using repo = await artifacts.get(row.repo_name);
  try {
    const result = await commitFilesToArtifactsRepo({
      repo,
      branch: row.default_branch,
      message: commitMessage,
      authorName: session.sub.slice(0, 16),
      authorEmail: 'docs@sorane.dev',
      expectedParentSha,
      changes: [{ path, content: bytes }],
    });
    const now = Math.floor(Date.now() / 1000);
    await env.DB.prepare('UPDATE doc_repos SET updated_at=? WHERE id=? AND issuer=? AND sub=?')
      .bind(now, row.id, env.ISSUER, session.sub)
      .run();
    return json(env, {
      path,
      size: bytes.length,
      commit: result.commitHash,
      etag: quoteEtag(result.commitHash),
    });
  } catch (err) {
    if (err instanceof ConflictError) fail(412, 'precondition_failed');
    throw err;
  }
}

export async function deleteRepoBlob(
  request: Request,
  env: RepoEnv,
  id: string,
  rawPath: string,
): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  const path = normalizeRepoPath(decodeURIComponent(rawPath));
  if (!path) fail(400, 'invalid_path');

  using repo = await artifacts.get(row.repo_name);
  const result = await commitFilesToArtifactsRepo({
    repo,
    branch: row.default_branch,
    message: `Delete ${path}`,
    authorName: session.sub.slice(0, 16),
    authorEmail: 'docs@sorane.dev',
    changes: [{ path, content: null }],
  });
  return json(env, { path, deleted: true, commit: result.commitHash });
}

export async function listRepoTokens(request: Request, env: RepoEnv, id: string): Promise<Response> {
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  using repo = await artifacts.get(row.repo_name);
  const tokens = await repo.listTokens();
  return json(env, tokens);
}

export async function createRepoToken(request: Request, env: RepoEnv, id: string): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  const body = (await request.json().catch(() => ({}))) as { scope?: 'read' | 'write'; ttl?: number };
  const scope = body.scope === 'read' ? 'read' : 'write';
  const ttl = typeof body.ttl === 'number' && Number.isFinite(body.ttl) ? Math.max(60, Math.min(31536000, Math.floor(body.ttl))) : 86400;

  using repo = await artifacts.get(row.repo_name);
  const info = await repo.info();
  const token = await repo.createToken(scope, ttl);
  return json(env, { ...token, remote: info.remote }, 201);
}

export async function revokeRepoToken(
  request: Request,
  env: RepoEnv,
  id: string,
  tokenId: string,
): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  using repo = await artifacts.get(row.repo_name);
  const revoked = await repo.revokeToken(decodeURIComponent(tokenId));
  return json(env, { revoked });
}

export async function forkRepo(request: Request, env: RepoEnv, id: string): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const sourceRow = await ownedRepo(env, session, id);
  const body = (await request.json().catch(() => ({}))) as { slug?: string; title?: string };
  const newSlug = sanitizeSlug(body.slug ?? `${sourceRow.slug}-fork`) ?? `${sourceRow.slug}-fork`;
  const newTitle = (body.title ?? `${sourceRow.title} (Fork)`).trim().slice(0, 100);
  const newId = crypto.randomUUID();
  const newRepoName = `doc-${newId.slice(0, 8)}-${newSlug}`;

  using srcRepo = await artifacts.get(sourceRow.repo_name);
  const forked = await srcRepo.fork(newRepoName, {
    description: sourceRow.description || newTitle,
  });

  const now = Math.floor(Date.now() / 1000);
  await env.DB.prepare(
    'INSERT INTO doc_repos(id,issuer,sub,repo_name,slug,title,description,default_branch,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)',
  )
    .bind(
      newId,
      env.ISSUER,
      session.sub,
      newRepoName,
      newSlug,
      newTitle,
      sourceRow.description,
      forked.defaultBranch,
      now,
      now,
    )
    .run();

  return json(env, { id: newId, slug: newSlug, repo_name: newRepoName, title: newTitle }, 201);
}

export async function deleteRepo(request: Request, env: RepoEnv, id: string): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  await artifacts.delete(row.repo_name);
  await env.DB.prepare('DELETE FROM doc_repos WHERE id=? AND issuer=? AND sub=?')
    .bind(row.id, env.ISSUER, session.sub)
    .run();
  return new Response(null, { status: 204, headers: baseHeaders(env) });
}

export async function reposHome(request: Request, env: RepoEnv): Promise<Response> {
  const { browser, fresh } = await ensureBrowser(request);
  const session = await current(request, env);
  const csrf = await hash(browser);
  if (!session) {
    return reposPage(
      env,
      'リポジトリ',
      `<div class="card"><div class="card-bd"><h2>文書の GitHub (Cloudflare Artifacts)</h2><p>Markdown や Office 文書を Git リポジトリとして管理し、AI エージェントの GitOps とブラウザ上の ibisOffice エディターをシームレスにつなぎます。</p><form method="post" action="/login">${formCsrf(csrf)}<button class="primary">mikaki でログイン</button></form></div></div>`,
      '/repos',
      200,
      fresh ? browser : undefined,
    );
  }

  const rows = await env.DB.prepare(
    'SELECT id,slug,title,description,default_branch,updated_at FROM doc_repos WHERE issuer=? AND sub=? ORDER BY updated_at DESC LIMIT 100',
  )
    .bind(env.ISSUER, session.sub)
    .all<Pick<RepoRow, 'id' | 'slug' | 'title' | 'description' | 'default_branch' | 'updated_at'>>();

  const items = (rows.results ?? [])
    .map((r) => {
      const when = new Date(r.updated_at * 1000).toLocaleString('ja-JP');
      return `<div class="row"><div><strong><a href="/repos/${escape(r.id)}">${escape(r.slug)}</a></strong> <span class="badge">${escape(r.default_branch)}</span><div class="muted">${escape(r.title)}${r.description ? ` — ${escape(r.description)}` : ''} · 更新: ${escape(when)}</div></div><div><a class="btn" href="/repos/${escape(r.id)}">開く</a></div></div>`;
    })
    .join('');

  const html = `
    <div class="grid2">
      <div class="card">
        <div class="card-hd"><strong>文書リポジトリ一覧</strong><span class="badge">${(rows.results ?? []).length} 件</span></div>
        ${items || '<div class="card-bd"><p class="muted">まだリポジトリがありません。右のフォームから作成してください。</p></div>'}
      </div>
      <div class="card">
        <div class="card-hd"><strong>新しいリポジトリを作成</strong></div>
        <div class="card-bd">
          <form id="create-repo">
            <label>スラッグ（半角英数・ハイフン）<input name="slug" placeholder="tech-strategy-2026" pattern="[A-Za-z0-9._-]{1,48}" required></label>
            <label>タイトル<input name="title" placeholder="2026年度 技術戦略レポート" maxlength="100" required></label>
            <label>説明（任意）<input name="description" placeholder="GitOps と ibisOffice で共同編集する文書群" maxlength="200"></label>
            <button class="primary" type="submit">リポジトリを作成 (README.md 付き)</button>
          </form>
          <p id="status" role="status"></p>
        </div>
      </div>
    </div>
    <script>
      const csrf = ${JSON.stringify(csrf)};
      const status = document.querySelector('#status');
      document.querySelector('#create-repo').addEventListener('submit', async (e) => {
        e.preventDefault();
        const f = e.currentTarget;
        status.textContent = '作成中…'; status.dataset.kind = '';
        try {
          const r = await fetch('/api/repos', {
            method: 'POST',
            credentials: 'same-origin',
            headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
            body: JSON.stringify({
              slug: f.slug.value.trim(),
              title: f.title.value.trim(),
              description: f.description.value.trim(),
              seedReadme: true,
            }),
          });
          const data = await r.json().catch(() => ({}));
          if (r.ok && data.id) {
            location.href = '/repos/' + data.id;
            return;
          }
          status.textContent = '作成できませんでした (' + (data.error || r.status) + ')';
          status.dataset.kind = 'error';
        } catch {
          status.textContent = '通信エラーが発生しました。';
          status.dataset.kind = 'error';
        }
      });
    </script>`;

  return reposPage(env, 'リポジトリ一覧', html, '/repos', 200, fresh ? browser : undefined);
}

export async function repoDetailPage(request: Request, env: RepoEnv, id: string): Promise<Response> {
  const { browser, fresh } = await ensureBrowser(request);
  const session = await current(request, env);
  const csrf = await hash(browser);
  if (!session) {
    return reposPage(
      env,
      'ログインが必要です',
      `<div class="card"><div class="card-bd"><p>リポジトリを表示するにはログインしてください。</p><form method="post" action="/login">${formCsrf(csrf)}<button class="primary">mikaki でログイン</button></form></div></div>`,
      `/repos/${id}`,
      200,
      fresh ? browser : undefined,
    );
  }

  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  const dirPath = normalizeRepoPath(new URL(request.url).searchParams.get('path') ?? '') ?? '';

  using repo = await artifacts.get(row.repo_name);
  const info = await repo.info();
  const { entries } = await readTreeAtPath(repo, row.default_branch, dirPath);
  const commits = await repo.log({ ref: row.default_branch, limit: 15 });

  const fileRows = entries
    .map((entry) => {
      const fullPath = dirPath ? `${dirPath}/${entry.name}` : entry.name;
      if (entry.type === 'tree') {
        const nextUrl = `/repos/${encodeURIComponent(row.id)}?path=${encodeURIComponent(fullPath)}`;
        return `<div class="row"><div>📁 <a href="${escape(nextUrl)}"><strong>${escape(entry.name)}/</strong></a></div><div class="muted">${escape(entry.hash.slice(0, 7))}</div></div>`;
      }
      const blobUrl = `/api/repos/${row.id}/blob/${fullPath.split('/').map(encodeURIComponent).join('/')}`;
      const editorUrl = `/editor.html?open=${encodeURIComponent(blobUrl)}&download=1`;
      const openBtn = isEditableInIbis(entry.name)
        ? `<a class="btn btn-primary" href="${escape(editorUrl)}">Docs で開く</a>`
        : '';
      return `<div class="row"><div>📄 <strong>${escape(entry.name)}</strong> <span class="muted">${escape(entry.hash.slice(0, 7))}</span></div><div style="display:flex;gap:.5rem">${openBtn}<a class="btn" href="${escape(blobUrl)}" target="_blank">Raw</a></div></div>`;
    })
    .join('');

  const commitRows = commits
    .map((c) => {
      const when = new Date(c.committedAt * 1000).toLocaleString('ja-JP');
      return `<div class="row"><div><strong>${escape(c.message)}</strong><div class="muted">${escape(c.author.name)} · ${escape(when)}</div></div><div><code class="badge">${escape(c.hash.slice(0, 7))}</code></div></div>`;
    })
    .join('');

  const parentLink = dirPath
    ? `<a class="btn" href="/repos/${escape(row.id)}${dirPath.includes('/') ? `?path=${encodeURIComponent(dirPath.slice(0, dirPath.lastIndexOf('/')))}` : ''}">.. (上へ)</a>`
    : '';

  const html = `
    <div style="display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:1rem;margin-bottom:1rem">
      <div>
        <h1 style="margin:0;font-size:1.35rem">${escape(row.slug)} <span class="badge">${escape(row.default_branch)}</span></h1>
        <div class="muted">${escape(row.title)}${row.description ? ` — ${escape(row.description)}` : ''}</div>
      </div>
      <div style="display:flex;gap:.5rem">
        <button id="fork-btn" type="button">Fork</button>
        <button id="delete-btn" class="danger" type="button">削除</button>
      </div>
    </div>

    <div class="card">
      <div class="card-hd">
        <strong>Git Clone / LLM GitOps</strong>
        <div style="display:flex;gap:.5rem">
          <button id="mint-read" type="button">Read トークン発行</button>
          <button id="mint-write" class="primary" type="button">Write トークン発行</button>
        </div>
      </div>
      <div class="card-bd">
        <pre class="code">git clone ${escape(info.remote)}</pre>
        <div id="token-box" style="margin-top:.6rem"></div>
      </div>
    </div>

    <div class="card">
      <div class="card-hd">
        <div><strong>ファイル一覧</strong> ${dirPath ? `<code>/${escape(dirPath)}</code>` : '<code>/</code>'} ${parentLink}</div>
        <span class="badge">${entries.length} 件</span>
      </div>
      ${fileRows || '<div class="card-bd"><p class="muted">ファイルはまだありません。</p></div>'}
    </div>

    <div class="card">
      <div class="card-hd"><strong>新しいファイルをコミット (Web / Upload)</strong></div>
      <div class="card-bd">
        <form id="commit-file">
          <div class="grid2">
            <label>パス（例: docs/proposal.md）<input name="path" placeholder="${dirPath ? `${escape(dirPath)}/` : ''}proposal.md" required></label>
            <label>コミットメッセージ<input name="message" placeholder="Add proposal.md" required></label>
          </div>
          <label>Markdown / テキスト内容（または下のファイル選択）<textarea name="text" rows="5" placeholder="# タイトル&#10;&#10;本文..."></textarea></label>
          <label>または Office / Markdown ファイルを選択<input type="file" name="file" accept=".md,.markdown,.docx,.xlsx,.pptx"></label>
          <button class="primary" type="submit">Commit &amp; Push</button>
        </form>
        <p id="status" role="status"></p>
      </div>
    </div>

    <div class="card">
      <div class="card-hd"><strong>コミット履歴</strong><span class="badge">${commits.length} 件</span></div>
      ${commitRows || '<div class="card-bd"><p class="muted">まだコミットがありません。</p></div>'}
    </div>

    <script>
      const csrf = ${JSON.stringify(csrf)};
      const repoId = ${JSON.stringify(row.id)};
      const status = document.querySelector('#status');
      const tokenBox = document.querySelector('#token-box');

      async function mint(scope) {
        tokenBox.textContent = 'トークンを発行中…';
        const r = await fetch('/api/repos/' + repoId + '/tokens', {
          method: 'POST',
          credentials: 'same-origin',
          headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
          body: JSON.stringify({ scope, ttl: 86400 }),
        });
        const d = await r.json().catch(() => ({}));
        if (!r.ok || !d.plaintext) {
          tokenBox.textContent = '発行に失敗しました。';
          return;
        }
        const cloneUrl = d.remote.replace('https://', 'https://x-access-token:' + d.plaintext + '@');
        tokenBox.innerHTML = '';
        const p = document.createElement('p');
        p.className = 'muted';
        p.textContent = 'スコープ: ' + d.scope + '（有効期限: ' + d.expiresAt + ' / この一度のみ表示）';
        const pre = document.createElement('pre');
        pre.className = 'code';
        pre.textContent = 'git clone ' + cloneUrl;
        tokenBox.append(p, pre);
      }
      document.querySelector('#mint-read').addEventListener('click', () => mint('read'));
      document.querySelector('#mint-write').addEventListener('click', () => mint('write'));

      document.querySelector('#fork-btn').addEventListener('click', async () => {
        const slug = prompt('フォーク先のスラッグを入力してください:', ${JSON.stringify(`${row.slug}-fork`)});
        if (!slug) return;
        const r = await fetch('/api/repos/' + repoId + '/fork', {
          method: 'POST',
          credentials: 'same-origin',
          headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
          body: JSON.stringify({ slug }),
        });
        const d = await r.json().catch(() => ({}));
        if (r.ok && d.id) location.href = '/repos/' + d.id;
        else alert('Fork に失敗しました');
      });

      document.querySelector('#delete-btn').addEventListener('click', async () => {
        if (!confirm('リポジトリ「${escape(row.slug)}」を削除しますか？Git 履歴も削除されます。')) return;
        const r = await fetch('/api/repos/' + repoId, {
          method: 'DELETE',
          credentials: 'same-origin',
          headers: { 'x-csrf-token': csrf },
        });
        if (r.status === 204) location.href = '/repos';
        else alert('削除できませんでした');
      });

      document.querySelector('#commit-file').addEventListener('submit', async (e) => {
        e.preventDefault();
        const f = e.currentTarget;
        const path = f.path.value.trim().replace(/^\\/+/, '');
        const msg = f.message.value.trim() || ('Update ' + path);
        const file = f.file.files[0];
        const body = file ? file : new Blob([f.text.value], { type: 'text/markdown' });
        status.textContent = 'コミット中…'; status.dataset.kind = '';
        try {
          const encodedPath = path.split('/').map(encodeURIComponent).join('/');
          const r = await fetch('/api/repos/' + repoId + '/blob/' + encodedPath, {
            method: 'PUT',
            credentials: 'same-origin',
            headers: {
              'x-csrf-token': csrf,
              'x-commit-message': encodeURIComponent(msg),
            },
            body,
          });
          if (r.ok) {
            status.textContent = 'コミットしました。再読み込みします…';
            status.dataset.kind = 'ok';
            location.reload();
            return;
          }
          const d = await r.json().catch(() => ({}));
          status.textContent = '失敗しました: ' + (d.error || r.status);
          status.dataset.kind = 'error';
        } catch {
          status.textContent = '通信エラーが発生しました。';
          status.dataset.kind = 'error';
        }
      });
    </script>`;

  return reposPage(env, row.slug, html, `/repos/${row.id}`, 200, fresh ? browser : undefined);
}

export async function handleReposRequest(request: Request, env: RepoEnv, path: string): Promise<Response | null> {
  if (request.method === 'GET' && path === '/repos') return await reposHome(request, env);
  const pageMatch = REPO_PAGE_PATH.exec(path);
  if (request.method === 'GET' && pageMatch) return await repoDetailPage(request, env, pageMatch[1]);

  if (request.method === 'GET' && path === '/api/repos') return await listRepos(request, env);
  if (request.method === 'POST' && path === '/api/repos') return await createRepo(request, env);

  const itemMatch = REPO_API_ITEM.exec(path);
  if (itemMatch) {
    if (request.method === 'GET') return await getRepoDetail(request, env, itemMatch[1]);
    if (request.method === 'DELETE') return await deleteRepo(request, env, itemMatch[1]);
  }

  const blobMatch = REPO_API_BLOB.exec(path);
  if (blobMatch) {
    if (request.method === 'GET' || request.method === 'HEAD') {
      return await readRepoBlob(request, env, blobMatch[1], blobMatch[2]);
    }
    if (request.method === 'PUT') {
      return await writeRepoBlob(request, env, blobMatch[1], blobMatch[2]);
    }
    if (request.method === 'DELETE') {
      return await deleteRepoBlob(request, env, blobMatch[1], blobMatch[2]);
    }
  }

  const tokensMatch = REPO_API_TOKENS.exec(path);
  if (tokensMatch) {
    if (request.method === 'GET') return await listRepoTokens(request, env, tokensMatch[1]);
    if (request.method === 'POST') return await createRepoToken(request, env, tokensMatch[1]);
  }

  const tokenItemMatch = REPO_API_TOKEN_ITEM.exec(path);
  if (tokenItemMatch && request.method === 'DELETE') {
    return await revokeRepoToken(request, env, tokenItemMatch[1], tokenItemMatch[2]);
  }

  const forkMatch = REPO_API_FORK.exec(path);
  if (forkMatch && request.method === 'POST') {
    return await forkRepo(request, env, forkMatch[1]);
  }

  return null;
}
