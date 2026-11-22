// SPDX-License-Identifier: AGPL-3.0-only
import {
  BROWSER,
  HttpError,
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
  type BranchRef,
  commitFilesToArtifactsRepo,
  createBranchInArtifactsRepo,
  isValidBranchName,
  listBranchesFromRemote,
  normalizeRepoPath,
  pushBranchToExternalRemote,
  validateExternalGitUrl,
} from './git-pack';
import {
  computeLineDiff,
  type TreeDiffEntry,
  diffCommitTrees,
  isTextDiffablePath,
  looksLikeBinary,
} from './diff';
import {
  USER_MAX_FILE,
  bareEtag,
  boundedBody,
  looksLikeOffice,
  quoteEtag,
} from './office-file';
import { NEXT, type FileEnv } from './files';
import { GitHubRepo, githubCoordinates, githubUrlCoordinates } from './github-repo';
import { isSpecSource, renderSpecSource, specSourceUrl } from './spec-source';

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

type ReadRepo = Pick<ArtifactsRepo, 'log' | 'readCommit' | 'readTree'> & {
  readBlob(hash: string): Promise<Pick<Blob, 'size' | 'arrayBuffer'> | null>;
};

const OPEN_REPO = `<div class="card"><div class="card-bd">
  <form method="get" action="/repos/open">
    <label>GitリポジトリのURL<input name="url" type="url" placeholder="https://github.com/owner/repo" required></label>
    <button type="submit">リポジトリを開く</button>
  </form>
</div></div>`;

const MAX_TEXT_FILE = 4 * 1024 * 1024;
const MAX_DIFF_FILE_BYTES = 512 * 1024;
const MAX_DIFF_FILES = 30;
const REPO_PAGE_PATH = /^\/repos\/([0-9a-f-]{36})$/i;
const REPO_API_ITEM = /^\/api\/repos\/([0-9a-f-]{36})$/i;
const REPO_API_BLOB = /^\/api\/repos\/([0-9a-f-]{36})\/blob\/(.+)$/i;
const REPO_API_BRANCHES = /^\/api\/repos\/([0-9a-f-]{36})\/branches$/i;
const REPO_API_DIFF = /^\/api\/repos\/([0-9a-f-]{36})\/diff$/i;
const REPO_API_PUSH = /^\/api\/repos\/([0-9a-f-]{36})\/push$/i;
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
    .source-line-number{display:inline-block;min-width:5ch;text-align:right;user-select:none}
    .source-code span:target{background:#fff8c5}
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
  repo: Pick<ReadRepo, 'log' | 'readTree'>,
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

function defaultSlugFromGitUrl(rawUrl: string): string {
  const cleaned = rawUrl.replace(/\/+$/, '').replace(/\.git$/i, '');
  const seg = cleaned.split('/').pop() || 'imported-repo';
  return sanitizeSlug(seg) ?? 'imported-repo';
}

function githubCompareUrl(remoteUrl: string, branch: string): string | null {
  try {
    const u = new URL(remoteUrl);
    if (u.hostname.toLowerCase() !== 'github.com') return null;
    const path = u.pathname.replace(/\/+$/, '').replace(/\.git$/i, '');
    if (!/^\/[^/]+\/[^/]+$/.test(path)) return null;
    return `https://github.com${path}/compare/${encodeURIComponent(branch)}?expand=1`;
  } catch {
    return null;
  }
}

export async function importRepo(request: Request, env: RepoEnv): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);

  const body = (await request.json().catch(() => ({}))) as {
    sourceUrl?: string;
    slug?: string;
    title?: string;
    description?: string;
    branch?: string;
    depth?: number;
  };
  const sourceUrl = typeof body.sourceUrl === 'string' ? validateExternalGitUrl(body.sourceUrl) : null;
  if (!sourceUrl) fail(400, 'invalid_url');

  const slug =
    typeof body.slug === 'string' && body.slug.trim()
      ? sanitizeSlug(body.slug)
      : defaultSlugFromGitUrl(sourceUrl);
  if (!slug) fail(400, 'invalid_slug');

  const branch = typeof body.branch === 'string' && body.branch.trim() ? body.branch.trim() : undefined;
  if (branch && !isValidBranchName(branch)) fail(400, 'invalid_branch');
  const depth =
    typeof body.depth === 'number' && Number.isFinite(body.depth) && body.depth >= 1
      ? Math.min(100, Math.floor(body.depth))
      : undefined;

  const title = (typeof body.title === 'string' ? body.title.trim() : '') || slug;
  if (title.length > 100) fail(400, 'title_too_long');
  const description = (typeof body.description === 'string' ? body.description.trim() : '').slice(0, 200);

  const id = crypto.randomUUID();
  const repoName = `${slug}-${id.slice(0, 8)}`;

  let imported: ArtifactsCreateRepoResult;
  try {
    imported = await artifacts.import({
      source: { url: sourceUrl, branch, depth },
      target: { name: repoName, opts: { description: description || undefined } },
    });
  } catch (err) {
    const code = (err as { code?: string })?.code;
    if (code === 'REMOTE_AUTH_REQUIRED' || code === 'INVALID_URL' || code === 'INVALID_INPUT') {
      fail(400, code.toLowerCase());
    }
    if (code === 'NOT_FOUND') fail(404, 'remote_not_found');
    if (code === 'MEMORY_LIMIT') fail(413, 'repo_too_large');
    if (code === 'UPSTREAM_UNAVAILABLE') fail(502, 'upstream_unavailable');
    throw err;
  }

  const defaultBranch = imported.defaultBranch || branch || 'main';
  const now = Math.floor(Date.now() / 1000);
  await env.DB.prepare(
    'INSERT INTO doc_repos(id,issuer,sub,repo_name,slug,title,description,default_branch,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)',
  )
    .bind(id, env.ISSUER, session.sub, repoName, slug, title, description, defaultBranch, now, now)
    .run();

  return json(
    env,
    {
      id,
      slug,
      repo_name: repoName,
      title,
      description,
      default_branch: defaultBranch,
      remote: imported.remote,
      source: sourceUrl,
    },
    201,
  );
}

export async function pushRepoToExternal(
  request: Request,
  env: RepoEnv,
  id: string,
): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);

  const body = (await request.json().catch(() => ({}))) as {
    remoteUrl?: string;
    branch?: string;
    remoteBranch?: string;
    token?: string;
    force?: boolean;
  };
  const remoteUrl = typeof body.remoteUrl === 'string' ? validateExternalGitUrl(body.remoteUrl) : null;
  if (!remoteUrl) fail(400, 'invalid_url');

  const localBranch =
    typeof body.branch === 'string' && body.branch.trim() ? body.branch.trim() : row.default_branch;
  if (!isValidBranchName(localBranch)) fail(400, 'invalid_branch');
  const remoteBranch =
    typeof body.remoteBranch === 'string' && body.remoteBranch.trim()
      ? body.remoteBranch.trim()
      : localBranch;
  if (!isValidBranchName(remoteBranch)) fail(400, 'invalid_branch');
  const token = typeof body.token === 'string' ? body.token.trim() : '';
  const force = body.force === true;

  using repo = await artifacts.get(row.repo_name);
  try {
    const result = await pushBranchToExternalRemote({
      repo,
      localBranch,
      remoteUrl,
      remoteBranch,
      token,
      force,
    });
    return json(env, {
      ...result,
      compareUrl: githubCompareUrl(remoteUrl, remoteBranch),
    });
  } catch (err) {
    if (err instanceof ConflictError) fail(409, 'non_fast_forward');
    if (err instanceof Error && err.message.startsWith('Local branch not found')) {
      fail(404, 'branch_not_found');
    }
    if (err instanceof Error && err.message.includes('(401)')) fail(401, 'remote_auth_failed');
    if (err instanceof Error && err.message.includes('(403)')) fail(403, 'remote_forbidden');
    throw err;
  }
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

  const params = new URL(request.url).searchParams;
  const ref = params.get('ref')?.trim() || row.default_branch;
  using repo = await artifacts.get(row.repo_name);
  const commits = await repo.log({ ref, limit: 1 });
  const headHash = commits[0]?.hash ?? '';
  const blob = await repo.readFile({ ref, path });
  if (!blob) fail(404, 'not_found');

  const headers = baseHeaders(env);
  headers.set('Content-Type', params.get('plain') === '1' && isTextDiffablePath(path)
    ? 'text/plain; charset=utf-8'
    : contentTypeForPath(path));
  const fileName = path.split('/').pop() ?? 'document';
  const asciiName = fileName.replace(/[^ -~]|["\\]/g, '_');
  const encodedName = encodeURIComponent(fileName).replace(/['()*]/g, (char) => '%' + char.charCodeAt(0).toString(16));
  headers.set('Content-Disposition', `inline; filename="${asciiName}"; filename*=UTF-8''${encodedName}`);
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

  const url = new URL(request.url);
  const rawBranch =
    url.searchParams.get('ref')?.trim() ||
    request.headers.get('x-target-branch')?.trim() ||
    row.default_branch;
  if (!isValidBranchName(rawBranch)) fail(400, 'invalid_branch');
  const rawBaseRef = request.headers.get('x-base-ref')?.trim() || row.default_branch;
  if (!isValidBranchName(rawBaseRef)) fail(400, 'invalid_branch');

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
      branch: rawBranch,
      baseRef: rawBaseRef,
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
      branch: rawBranch,
      size: bytes.length,
      commit: result.commitHash,
      etag: quoteEtag(result.commitHash),
    });
  } catch (err) {
    if (err instanceof ConflictError) fail(412, 'precondition_failed');
    throw err;
  }
}

async function resolveCommit(
  repo: Pick<ReadRepo, 'log' | 'readCommit'>,
  refOrSha: string,
): Promise<ArtifactsCommitMetadata | null> {
  const trimmed = refOrSha.trim();
  if (!trimmed) return null;
  if (/^[0-9a-f]{40}$/i.test(trimmed)) {
    return await repo.readCommit(trimmed.toLowerCase());
  }
  const logs = await repo.log({ ref: trimmed, limit: 1 });
  return logs[0] ?? null;
}

async function resolveBranches(repo: ArtifactsRepo, defaultBranch: string): Promise<BranchRef[]> {
  try {
    const info = await repo.info();
    const token = await repo.createToken('read', 60);
    try {
      const refs = await listBranchesFromRemote(info.remote, token.plaintext);
      if (refs.length > 0) return refs;
    } finally {
      await repo.revokeToken(token.id).catch(() => {});
    }
  } catch {
    // Fall back to default_branch log if remote ref discovery is unavailable.
  }
  const head = (await repo.log({ ref: defaultBranch, limit: 1 }))[0];
  return head ? [{ name: defaultBranch, sha: head.hash }] : [];
}

export async function listRepoBranches(
  request: Request,
  env: RepoEnv,
  id: string,
): Promise<Response> {
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  using repo = await artifacts.get(row.repo_name);
  const refs = await resolveBranches(repo, row.default_branch);
  return json(env, {
    defaultBranch: row.default_branch,
    branches: refs.map((b) => ({
      ...b,
      isDefault: b.name === row.default_branch,
    })),
  });
}

export async function createRepoBranch(
  request: Request,
  env: RepoEnv,
  id: string,
): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  const body = (await request.json().catch(() => ({}))) as { name?: string; fromRef?: string };
  const branch = typeof body.name === 'string' ? body.name.trim() : '';
  if (!isValidBranchName(branch)) fail(400, 'invalid_branch');
  const fromRef =
    typeof body.fromRef === 'string' && body.fromRef.trim() ? body.fromRef.trim() : row.default_branch;

  using repo = await artifacts.get(row.repo_name);
  try {
    const created = await createBranchInArtifactsRepo({ repo, branch, fromRef });
    return json(env, created, 201);
  } catch (err) {
    if (err instanceof ConflictError) fail(409, 'branch_exists');
    if (err instanceof Error && err.message.startsWith('Source ref not found')) {
      fail(404, 'ref_not_found');
    }
    throw err;
  }
}

export async function diffRepo(request: Request, env: RepoEnv, id: string): Promise<Response> {
  const session = await requireUser(request, env);
  const artifacts = requireArtifacts(env);
  const row = await ownedRepo(env, session, id);
  using repo = await artifacts.get(row.repo_name);
  return diffRepoResponse(request, env, repo, row.default_branch);
}

async function diffRepoResponse(request: Request, env: RepoEnv, repo: ReadRepo, defaultBranch: string) {
  const url = new URL(request.url);
  const headRef = url.searchParams.get('head')?.trim() || defaultBranch;
  const baseParam = url.searchParams.get('base')?.trim() || '';
  const rawPathFilter = url.searchParams.get('path')?.trim() || '';
  const pathFilter = rawPathFilter ? normalizeRepoPath(rawPathFilter) : null;
  if (rawPathFilter && !pathFilter) fail(400, 'invalid_path');

  const headCommit = await resolveCommit(repo, headRef);
  if (!headCommit) fail(404, 'head_not_found');

  let baseCommit: ArtifactsCommitMetadata | null = null;
  if (baseParam) {
    baseCommit = await resolveCommit(repo, baseParam);
    if (!baseCommit) fail(404, 'base_not_found');
  } else if (headCommit.parents.length > 0) {
    baseCommit = await resolveCommit(repo, headCommit.parents[0]);
  }

  let changes: TreeDiffEntry[];
  if (pathFilter) {
    const readFileEntry = treeFileReader(repo, pathFilter);
    const [before, after] = await Promise.all([
      readFileEntry(baseCommit?.treeHash),
      readFileEntry(headCommit.treeHash),
    ]);
    changes = sameFileEntry(before, after) ? [] : [{
      path: pathFilter,
      status: !before ? 'added' : !after ? 'deleted' : 'modified',
      oldHash: before?.hash ?? null,
      newHash: after?.hash ?? null,
    }];
  } else {
    changes = await diffCommitTrees(repo, baseCommit?.treeHash ?? null, headCommit.treeHash);
  }

  const totalChanged = changes.length;
  const truncatedList = totalChanged > MAX_DIFF_FILES;
  const sliced = changes.slice(0, MAX_DIFF_FILES);
  const textDecoder = new TextDecoder();

  const files = await Promise.all(
    sliced.map(async (entry) => {
      const oldBlob = entry.oldHash ? await repo.readBlob(entry.oldHash) : null;
      const newBlob = entry.newHash ? await repo.readBlob(entry.newHash) : null;
      const oldSize = oldBlob ? oldBlob.size : 0;
      const newSize = newBlob ? newBlob.size : 0;
      const withinSize = oldSize <= MAX_DIFF_FILE_BYTES && newSize <= MAX_DIFF_FILE_BYTES;
      let isText = isTextDiffablePath(entry.path) && withinSize;
      let oldText = '';
      let newText = '';

      if (isText) {
        const oldBytes = oldBlob ? new Uint8Array(await oldBlob.arrayBuffer()) : new Uint8Array(0);
        const newBytes = newBlob ? new Uint8Array(await newBlob.arrayBuffer()) : new Uint8Array(0);
        if (looksLikeBinary(oldBytes) || looksLikeBinary(newBytes)) {
          isText = false;
        } else {
          oldText = textDecoder.decode(oldBytes);
          newText = textDecoder.decode(newBytes);
        }
      }

      if (!isText) {
        return {
          path: entry.path,
          status: entry.status,
          isText: false,
          oldSize,
          newSize,
          additions: 0,
          deletions: 0,
          hunks: [],
          truncated: !withinSize,
        };
      }

      const diff = computeLineDiff(oldText, newText);
      return {
        path: entry.path,
        status: entry.status,
        isText: true,
        oldSize,
        newSize,
        additions: diff.additions,
        deletions: diff.deletions,
        hunks: diff.hunks,
        truncated: diff.truncated,
      };
    }),
  );

  return json(env, {
    baseRef: baseParam || (baseCommit ? baseCommit.hash : null),
    headRef,
    baseCommit: baseCommit
      ? {
          hash: baseCommit.hash,
          message: baseCommit.message,
          author: baseCommit.author,
          committedAt: baseCommit.committedAt,
        }
      : null,
    headCommit: {
      hash: headCommit.hash,
      message: headCommit.message,
      author: headCommit.author,
      committedAt: headCommit.committedAt,
      parents: headCommit.parents,
    },
    totalChanged,
    truncated: truncatedList,
    files,
  });
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
      OPEN_REPO + `<div class="card"><div class="card-bd"><h2>Docsでリポジトリを管理</h2><form method="post" action="/login">${formCsrf(csrf)}<button class="primary">mikaki でログイン</button></form></div></div>`,
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

  const html = OPEN_REPO + `
    <div class="grid2">
      <div class="card">
        <div class="card-hd"><strong>文書リポジトリ一覧</strong><span class="badge">${(rows.results ?? []).length} 件</span></div>
        ${items || '<div class="card-bd"><p class="muted">まだリポジトリがありません。右のフォームから作成してください。</p></div>'}
      </div>
      <div>
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
        <div class="card">
          <div class="card-hd"><strong>外部 Git リポジトリからインポート</strong></div>
          <div class="card-bd">
            <form id="import-repo">
              <label>GitHub / HTTPS リポジトリ URL<input name="sourceUrl" type="url" placeholder="https://github.com/owner/repo.git" required></label>
              <div class="grid2">
                <label>スラッグ（任意）<input name="slug" placeholder="my-docs" pattern="[A-Za-z0-9._-]{1,48}"></label>
                <label>ブランチ（任意）<input name="branch" placeholder="main"></label>
              </div>
              <button type="submit">Import</button>
            </form>
            <p id="import-status" role="status"></p>
            <div id="houritsu-browse" hidden>
              <a href="https://law.sorane.dev/">法令ビューアで二版を比較</a>
              · <a href="https://github.com/Kohei-Wada/houritsu/commits/main/">GitHubで改正履歴を見る</a>
            </div>
          </div>
        </div>
      </div>
    </div>
    <script>
      const csrf = ${JSON.stringify(csrf)};
      const status = document.querySelector('#status');
      const importStatus = document.querySelector('#import-status');
      const houritsuBrowse = document.querySelector('#houritsu-browse');
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
      document.querySelector('#import-repo').addEventListener('submit', async (e) => {
        e.preventDefault();
        const f = e.currentTarget;
        importStatus.textContent = 'インポート中…'; importStatus.dataset.kind = '';
        houritsuBrowse.hidden = true;
        try {
          const sourceUrl = f.sourceUrl.value.trim();
          const r = await fetch('/api/repos/import', {
            method: 'POST',
            credentials: 'same-origin',
            headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
            body: JSON.stringify({
              sourceUrl,
              slug: f.slug.value.trim(),
              branch: f.branch.value.trim(),
              depth: 10,
            }),
          });
          const data = await r.json().catch(() => ({}));
          if (r.ok && data.id) {
            location.href = '/repos/' + data.id;
            return;
          }
          if (r.status === 413 && data.error === 'repo_too_large') {
            importStatus.textContent = 'Gitサービスが一度に取り込めるデータ量を超えました。';
            const source = new URL(sourceUrl);
            const path = source.pathname.toLowerCase();
            const repoPath = path.endsWith('/') ? path.slice(0, -1) : path;
            houritsuBrowse.hidden = source.hostname !== 'github.com' ||
              !['/kohei-wada/houritsu', '/kohei-wada/houritsu.git'].includes(repoPath);
          } else {
            importStatus.textContent = 'インポートできませんでした (' + (data.error || r.status) + ')';
          }
          importStatus.dataset.kind = 'error';
        } catch {
          importStatus.textContent = '通信エラーが発生しました。';
          importStatus.dataset.kind = 'error';
        }
      });
    </script>`;

  return reposPage(env, 'リポジトリ一覧', html, '/repos', 200, fresh ? browser : undefined);
}

const HISTORY_BATCH = 30;

function treeFileReader(repo: Pick<ReadRepo, 'readTree'>, path: string) {
  const segments = path.split('/');
  const trees = new Map<string, Promise<ArtifactsTreeEntry[] | null>>();
  return async (root: string | undefined) => {
    if (!root) return null;
    let hash = root;
    for (const [index, name] of segments.entries()) {
      let tree = trees.get(hash);
      if (!tree) {
        tree = repo.readTree(hash);
        trees.set(hash, tree);
      }
      const entry = (await tree)?.find((item) => item.name === name);
      if (!entry) return null;
      if (index === segments.length - 1) {
        return entry.type === 'blob' || entry.type === 'exec' ? entry : null;
      }
      if (entry.type !== 'tree') return null;
      hash = entry.hash;
    }
    return null;
  };
}

function sameFileEntry(before: ArtifactsTreeEntry | null, after: ArtifactsTreeEntry | null): boolean {
  return before?.hash === after?.hash && before?.mode === after?.mode;
}

async function readHistory(repo: ReadRepo, ref: string, path: string | null) {
  const commits = await repo.log({ ref, limit: HISTORY_BATCH + 1 });
  const fileVersion = path ? treeFileReader(repo, path) : null;
  const rows = [];
  for (const [index, commit] of commits.slice(0, HISTORY_BATCH).entries()) {
    if (!fileVersion) {
      rows.push({ commit, status: null });
      continue;
    }
    const next = commits[index + 1];
    const parent = next?.hash === commit.parents[0] ? next
      : commit.parents[0] ? await repo.readCommit(commit.parents[0]) : null;
    const [before, after] = await Promise.all([
      fileVersion(parent?.treeHash),
      fileVersion(commit.treeHash),
    ]);
    if (!sameFileEntry(before, after)) {
      rows.push({ commit, status: !before ? '追加' : !after ? '削除' : '変更' });
    }
  }
  return { rows, next: commits[HISTORY_BATCH]?.hash };
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
  using repo = await artifacts.get(row.repo_name);
  const info = await repo.info();
  const branches = new URL(request.url).searchParams.has('source') ? [] : await resolveBranches(repo, row.default_branch);
  return renderRepoDetail(request, env, repo, {
    id: row.id, slug: row.slug, title: row.title, description: row.description,
    defaultBranch: row.default_branch, info, branches, csrf, browser: fresh ? browser : undefined,
    repoUrl: '/repos/' + row.id, apiUrl: '/api/repos/' + row.id,
  });
}

async function renderRepoDetail(request: Request, env: RepoEnv, repo: ReadRepo, row: {
  id: string;
  slug: string;
  title: string;
  description: string | null;
  defaultBranch: string;
  info: { remote: string; source?: string | null };
  branches: BranchRef[];
  csrf?: string;
  browser?: string;
  repoUrl: string;
  apiUrl: string;
  external?: GitHubRepo;
}) {
  const { info, branches, csrf, repoUrl, apiUrl, external } = row;
  const url = new URL(request.url);
  const dirPath = normalizeRepoPath(url.searchParams.get('path') ?? '') ?? '';
  const activeRef = url.searchParams.get('ref')?.trim() || row.defaultBranch;
  const sourceParam = url.searchParams.get('source');
  if (sourceParam !== null) {
    const path = normalizeRepoPath(sourceParam);
    if (!path || !isSpecSource(path)) fail(400, 'invalid_source_path');
    const slash = path.lastIndexOf('/');
    const parent = slash < 0 ? '' : path.slice(0, slash);
    const name = path.slice(slash + 1);
    const { entries, headCommit } = await readTreeAtPath(repo, activeRef, parent);
    const entry = entries.find((entry) => entry.name === name && (entry.type === 'blob' || entry.type === 'exec'));
    if (!headCommit || !entry) fail(404, 'not_found');
    const blob = await repo.readBlob(entry.hash);
    if (!blob) fail(404, 'not_found');
    const commit = headCommit.hash;
    const rawUrl = apiUrl + '/blob/' + path.split('/').map(encodeURIComponent).join('/') + '?ref=' + commit + '&plain=1';
    const directoryUrl = repoUrl + '?' + new URLSearchParams({ ref: commit, ...(parent ? { path: parent } : {}) });
    let content = '<p>このファイルはソース表示の上限（512 KiB）を超えています。Raw で確認できます。</p>';
    if (blob.size <= MAX_DIFF_FILE_BYTES) {
      try {
        const bytes = new Uint8Array(await blob.arrayBuffer());
        if (looksLikeBinary(bytes)) fail(415, 'not_utf8_text');
        const text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes);
        content = renderSpecSource(path, text, repoUrl, commit);
      } catch (error) {
        if (error instanceof TypeError) fail(415, 'not_utf8_text');
        throw error;
      }
    }
    return reposPage(env, path, `<div class="card"><div class="card-hd"><div><strong>${escape(path)}</strong>
      <span class="muted">${escape(row.slug)} · <code>${escape(commit.slice(0, 12))}</code></span></div>
      <div><a class="btn" href="${escape(directoryUrl)}">ファイル一覧</a> <a class="btn" href="${escape(rawUrl)}">Raw</a></div>
      </div><div class="card-bd">${content}</div></div>`, repoUrl, 200, row.browser);
  }
  if (!branches.some((b) => b.name === activeRef)) {
    branches.unshift({ name: activeRef, sha: '' });
  }
  const { entries, headCommit } = await readTreeAtPath(repo, activeRef, dirPath);
  if (external && !headCommit) fail(404, 'not_found');
  const rawHistoryPath = url.searchParams.get('file');
  const historyPath = rawHistoryPath === null ? null : normalizeRepoPath(rawHistoryPath);
  const cursor = url.searchParams.get('before');
  if (rawHistoryPath !== null && !historyPath) fail(400, 'invalid_path');
  const anchor = url.searchParams.get('anchor');
  if (cursor !== null && !(external ? /^[1-9][0-9]{0,5}$/ : /^[0-9a-f]{40}$/).test(cursor)) fail(400, 'invalid_commit');
  if (anchor !== null && !/^[0-9a-f]{40}$/.test(anchor)) fail(400, 'invalid_commit');
  const history = external && headCommit
    ? await external.history(cursor && anchor ? anchor : headCommit.hash, historyPath, cursor ? Number(cursor) : 1)
    : await readHistory(repo, cursor ?? activeRef, historyPath);
  const pageParams = new URLSearchParams({ ref: activeRef });
  if (dirPath) pageParams.set('path', dirPath);
  const compareBase = url.searchParams.get('compareBase') ?? '';
  const compareHead = url.searchParams.get('compareHead') ?? '';
  function historyUrl(file: string | null, before?: string) {
    const params = new URLSearchParams(pageParams);
    if (file) params.set('file', file);
    if (before) {
      params.set('before', before);
      if (external && headCommit) params.set('anchor', cursor && anchor ? anchor : headCommit.hash);
    }
    if (file === historyPath) {
      if (compareBase) params.set('compareBase', compareBase);
      if (compareHead) params.set('compareHead', compareHead);
    }
    return repoUrl + '?' + params.toString() + '#commit-history';
  }

  const refQuery = external && headCommit ? '?ref=' + headCommit.hash
    : activeRef !== row.defaultBranch ? `?ref=${encodeURIComponent(activeRef)}` : '';

  const fileRows = entries
    .map((entry) => {
      const fullPath = dirPath ? `${dirPath}/${entry.name}` : entry.name;
      if (entry.type === 'tree') {
        const nextUrl = `${repoUrl}?path=${encodeURIComponent(fullPath)}${activeRef !== row.defaultBranch ? `&ref=${encodeURIComponent(activeRef)}` : ''}`;
        return `<div class="row"><div>📁 <a href="${escape(nextUrl)}"><strong>${escape(entry.name)}/</strong></a></div><div class="muted">${escape(entry.hash.slice(0, 7))}</div></div>`;
      }
      const blobBase = `${apiUrl}/blob/${fullPath.split('/').map(encodeURIComponent).join('/')}`;
      const blobUrl = `${blobBase}${refQuery}`;
      const editorUrl = `/editor.html?open=${encodeURIComponent(blobUrl)}&${external ? 'view=1' : 'download=1'}`;
      const openBtn = isEditableInIbis(entry.name)
        ? `<a class="btn btn-primary" href="${escape(editorUrl)}">Docs で開く</a>`
        : '';
      const sourceBtn = headCommit && isSpecSource(fullPath) && (entry.type === 'blob' || entry.type === 'exec')
        ? `<a class="btn" href="${escape(specSourceUrl(repoUrl, headCommit.hash, fullPath))}">ソース</a>` : '';
      const diffBtn = `<a class="btn" href="${escape(historyUrl(fullPath))}">履歴</a>`;
      return `<div class="row"><div>📄 <strong>${escape(entry.name)}</strong> <span class="muted">${escape(entry.hash.slice(0, 7))}</span></div><div style="display:flex;gap:.5rem;flex-wrap:wrap">${openBtn}${sourceBtn}${diffBtn}<a class="btn" href="${escape(blobUrl)}" target="_blank">Raw</a></div></div>`;
    })
    .join('');

  const commitRows = history.rows
    .map(({ commit: c, status }) => {
      const when = new Date(c.committedAt * 1000).toLocaleString('ja-JP', { timeZone: 'Asia/Tokyo' });
      const snapshot = repoUrl + '?' + new URLSearchParams({ ref: c.hash });
      const raw = historyPath && status !== '削除'
        ? apiUrl + '/blob/' + historyPath.split('/').map(encodeURIComponent).join('/') + '?ref=' + c.hash + '&plain=1'
        : null;
      const [subject, ...details] = c.message.split('\n');
      const message = details.join('\n').trim();
      return `<div class="row">
        <div><strong>${escape(subject)}</strong>${status ? ` <span class="badge">${status}</span>` : ''}
          <div class="muted">${escape(when)} · ${escape(c.author.name)} · <a href="${escape(snapshot)}"><code>${escape(c.hash.slice(0, 12))}</code></a></div>
          ${message ? `<details><summary>コミットメッセージ</summary><pre style="white-space:pre-wrap">${escape(message)}</pre></details>` : ''}
          ${c.parents.length > 1 ? `<div class="muted">親コミット ${c.parents.map((parent) => `<a href="${escape(repoUrl + '?' + new URLSearchParams({ ref: parent }))}">${escape(parent.slice(0, 12))}</a>`).join(' · ')}</div>` : ''}
        </div>
        <div style="display:flex;gap:.5rem;flex-wrap:wrap">
          ${raw ? `<a class="btn" href="${escape(raw)}">この版の本文</a>` : ''}
          <button type="button" class="diff-commit-btn" data-commit="${escape(c.hash)}">親と比較</button>
          <button type="button" data-set-base="${escape(c.hash)}">旧に設定</button>
          <button type="button" data-set-head="${escape(c.hash)}">新に設定</button>
        </div></div>`;
    })
    .join('');

  const branchOptions = branches
    .map(
      (b) =>
        `<option value="${escape(b.name)}"${b.name === activeRef ? ' selected' : ''}>${escape(b.name)}${b.name === row.defaultBranch ? ' (default)' : ''}</option>`,
    )
    .join('');

  const compareOptions = [
    ...branches.map((b) => `<option value="${escape(b.name)}">${escape(b.name)}</option>`),
    ...history.rows.map(
      ({ commit: c }) =>
        `<option value="${escape(c.hash)}">${escape(c.hash.slice(0, 7))} · ${escape(c.message.slice(0, 32))}</option>`,
    ),
  ].join('');

  const parentLink = dirPath
    ? `<a class="btn" href="${escape(repoUrl)}?${[dirPath.includes('/') ? `path=${encodeURIComponent(dirPath.slice(0, dirPath.lastIndexOf('/')))}` : '', activeRef !== row.defaultBranch ? `ref=${encodeURIComponent(activeRef)}` : ''].filter(Boolean).join('&')}">.. (上へ)</a>`
    : '';

  const html = `
    <div style="display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:1rem;margin-bottom:1rem">
      <div>
        <h1 style="margin:0;font-size:1.35rem">${escape(row.slug)} <span class="badge">${escape(activeRef)}</span></h1>
        <div class="muted">${escape(row.title)}${row.description ? `${row.title ? ' — ' : ''}${escape(row.description)}` : ''}</div>
      </div>
      <div style="display:flex;gap:.5rem;align-items:center;flex-wrap:wrap">
        <select id="branch-select" aria-label="ブランチ切り替え" style="width:auto">${branchOptions}</select>
        ${external ? `<a class="btn" href="${escape(info.remote)}">GitHub</a><span class="badge">読み取り専用</span>` : `
        <button id="new-branch-btn" type="button">+ ブランチ作成</button>
        <button id="fork-btn" type="button">Fork</button>
        <button id="delete-btn" class="danger" type="button">削除</button>`}
      </div>
    </div>

    ${external ? '' : `
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
        ${info.source ? `<div class="muted" style="margin-top:.4rem">Import / Fork 元: <code>${escape(info.source)}</code></div>` : ''}
        <div id="token-box" style="margin-top:.6rem"></div>
      </div>
    </div>

    <div class="card">
      <div class="card-hd"><strong>外部 Git リモートへ Push (GitHub 等)</strong></div>
      <div class="card-bd">
        <form id="push-external">
          <div class="grid2">
            <label>リモート URL<input name="remoteUrl" type="url" value="${escape(info.source && info.source.startsWith('https://') ? info.source : info.source && info.source.startsWith('github:') ? `https://github.com/${info.source.slice(7)}.git` : '')}" placeholder="https://github.com/owner/repo.git" required></label>
            <label>Push 先ブランチ<input name="remoteBranch" value="${escape(activeRef)}" required></label>
          </div>
          <label>Personal Access Token (PAT) <small>※サーバーには保存せず、この Push 通信のみに使用します</small><input name="token" type="password" placeholder="github_pat_..."></label>
          <button class="primary" type="submit">外部リモートへ Push</button>
        </form>
        <div id="push-status" style="margin-top:.6rem"></div>
      </div>
    </div>

    `}

    <div class="card">
      <div class="card-hd">
        <div><strong>ファイル一覧</strong> ${dirPath ? `<code>/${escape(dirPath)}</code>` : '<code>/</code>'} ${parentLink}</div>
        <span class="badge">${entries.length} 件</span>
      </div>
      ${fileRows || '<div class="card-bd"><p class="muted">ファイルはまだありません。</p></div>'}
    </div>

    ${external ? '' : `
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

    `}

    <div class="card" id="diff-card">
      <div class="card-hd">
        <strong>${historyPath ? escape(historyPath) + ' の比較' : '新旧比較 (Diff)'}</strong>
        <div style="display:flex;gap:.5rem;align-items:center;flex-wrap:wrap">
          <label style="margin:0;font-weight:400;display:inline-flex;align-items:center;gap:.3rem">旧 <input id="diff-base" list="diff-refs" placeholder="新の親コミット" value="${escape(compareBase)}"></label>
          <label style="margin:0;font-weight:400;display:inline-flex;align-items:center;gap:.3rem">新 <input id="diff-head" list="diff-refs" value="${escape(compareHead || activeRef)}"></label>
          <datalist id="diff-refs">${compareOptions}</datalist>
          <button id="run-diff-btn" type="button">比較する</button>
        </div>
      </div>
      <div class="card-bd" id="diff-body">
        <p class="muted">履歴から旧版・新版を選ぶか、ブランチ名・コミットSHAを入力してください。</p>
      </div>
    </div>

    <div class="card" id="commit-history">
      <div class="card-hd"><strong>${historyPath ? escape(historyPath) + ' の履歴' : 'コミット履歴'}</strong>
        ${historyPath || cursor ? `<a class="btn" data-history-page href="${escape(historyUrl(historyPath))}">最新へ</a>` : ''}
        ${historyPath ? `<a class="btn" href="${escape(historyUrl(null))}">全体の履歴</a>` : ''}
      </div>
      <div class="card-bd">
        <form id="history-ref-form" method="get" action="${escape(repoUrl)}#commit-history" style="display:flex;gap:.5rem;align-items:end;flex-wrap:wrap">
          <label>履歴の起点 <input name="ref" required placeholder="ブランチ名またはコミットSHA" value="${escape(activeRef)}"></label>
          ${historyPath ? `<input type="hidden" name="file" value="${escape(historyPath)}">` : ''}
          <input type="hidden" name="compareBase" value="${escape(compareBase)}">
          <input type="hidden" name="compareHead" value="${escape(compareHead)}">
          <button type="submit">移動</button>
        </form>
        <p><a href="#diff-card">比較欄へ</a> <span id="history-selection" class="muted" role="status"></span></p>
        <p class="muted">${external ? 'GitHubのコミット履歴を表示します。' : '選択した版から第1親をたどります。ファイルの履歴は同じパスの追加・変更・削除を表示します。'}</p>
      </div>
      ${commitRows || '<div class="card-bd"><p class="muted">この範囲に該当するコミットはありません。</p></div>'}
      ${history.next ? `<div class="card-bd"><a class="btn" data-history-page href="${escape(historyUrl(historyPath, history.next))}">さらに古い履歴</a></div>` : ''}
    </div>

    <style>
      .diff-file{border:1px solid var(--border);border-radius:6px;margin-bottom:.75rem;overflow:hidden}
      .diff-file-hd{background:#f6f8fa;padding:.45rem .75rem;display:flex;justify-content:space-between;align-items:center;gap:.5rem;border-bottom:1px solid var(--border);flex-wrap:wrap}
      .diff-table{width:100%;border-collapse:collapse;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.8rem}
      .diff-table td{padding:1px 8px;white-space:pre-wrap;word-break:break-word;vertical-align:top}
      .diff-ln{width:3.2rem;text-align:right;color:var(--muted);user-select:none;border-right:1px solid var(--border)}
      .diff-row-add{background:#e6ffec}
      .diff-row-del{background:#ffebe9}
      .diff-row-hunk{background:#ddf4ff;color:#0969da}
    </style>

    <script>
      const csrf = ${JSON.stringify(csrf)};
      const repoId = ${JSON.stringify(row.id)};
      const apiUrl = ${JSON.stringify(apiUrl)};
      const activeRef = ${JSON.stringify(activeRef).replace(/</g, '\\u003c')};
      const historyPath = ${JSON.stringify(historyPath).replace(/</g, '\\u003c')};
      const defaultBranch = ${JSON.stringify(row.defaultBranch)};
      const status = document.querySelector('#status');
      const tokenBox = document.querySelector('#token-box');
      const diffBody = document.querySelector('#diff-body');
      const diffBase = document.querySelector('#diff-base');
      const diffHead = document.querySelector('#diff-head');

      document.querySelector('#branch-select').addEventListener('change', (e) => {
        const branch = e.currentTarget.value;
        const u = new URL(location.href);
        u.searchParams.delete('before');
        u.searchParams.delete('anchor');
        if (branch === defaultBranch) u.searchParams.delete('ref');
        else u.searchParams.set('ref', branch);
        location.href = u.toString();
      });

      document.querySelector('#new-branch-btn')?.addEventListener('click', async () => {
        const name = prompt('新しいブランチ名を入力してください（例: draft/review-1）:');
        if (!name) return;
        const r = await fetch('/api/repos/' + repoId + '/branches', {
          method: 'POST',
          credentials: 'same-origin',
          headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
          body: JSON.stringify({ name: name.trim(), fromRef: activeRef }),
        });
        if (r.ok) {
          const u = new URL(location.href);
          u.searchParams.set('ref', name.trim());
          location.href = u.toString();
        } else {
          const d = await r.json().catch(() => ({}));
          alert('ブランチ作成に失敗しました: ' + (d.error || r.status));
        }
      });

      function renderDiffResponse(data) {
        diffBody.innerHTML = '';
        const summary = document.createElement('p');
        summary.className = 'muted';
        const baseLabel = data.baseCommit ? data.baseCommit.hash.slice(0, 7) : '(初期)';
        const headLabel = data.headCommit ? data.headCommit.hash.slice(0, 7) : data.headRef;
        summary.textContent = baseLabel + ' → ' + headLabel + ' · ' + data.files.length + ' ファイル変更';
        diffBody.append(summary);

        if (!data.files.length) {
          const empty = document.createElement('p');
          empty.className = 'muted';
          empty.textContent = '変更されたファイルはありません。';
          diffBody.append(empty);
          return;
        }

        const statusLabel = { added: '追加', modified: '変更', deleted: '削除' };
        for (const f of data.files) {
          const box = document.createElement('div');
          box.className = 'diff-file';
          const hd = document.createElement('div');
          hd.className = 'diff-file-hd';
          const left = document.createElement('div');
          const strong = document.createElement('strong');
          strong.textContent = f.path + ' ';
          const badge = document.createElement('span');
          badge.className = 'badge';
          badge.textContent = (statusLabel[f.status] || f.status) + (f.isText ? (' +' + f.additions + ' -' + f.deletions) : (' ' + f.oldSize + 'B → ' + f.newSize + 'B'));
          left.append(strong, badge);

          const right = document.createElement('div');
          if (f.status !== 'deleted' && /\\.(md|markdown|docx|xlsx|pptx)$/i.test(f.path) && data.headCommit) {
            const blobRefUrl = apiUrl + '/blob/' + f.path.split('/').map(encodeURIComponent).join('/') + '?ref=' + encodeURIComponent(data.headCommit.hash);
            const a = document.createElement('a');
            a.className = 'btn';
            a.href = '/editor.html?open=' + encodeURIComponent(blobRefUrl) + '&view=1';
            a.textContent = 'この版を Docs でプレビュー';
            right.append(a);
          }
          hd.append(left, right);
          box.append(hd);

          if (!f.isText) {
            const p = document.createElement('p');
            p.className = 'muted';
            p.style.padding = '.5rem .75rem';
            p.textContent = f.truncated ? 'ファイルサイズが大きいため差分表示を省略しました。' : 'バイナリファイル（サイズ: ' + f.oldSize + ' → ' + f.newSize + ' bytes）';
            box.append(p);
          } else if (!f.hunks.length) {
            const p = document.createElement('p');
            p.className = 'muted';
            p.style.padding = '.5rem .75rem';
            p.textContent = 'テキスト差分なし';
            box.append(p);
          } else {
            const table = document.createElement('table');
            table.className = 'diff-table';
            for (const h of f.hunks) {
              const hr = document.createElement('tr');
              hr.className = 'diff-row-hunk';
              const htd = document.createElement('td');
              htd.colSpan = 3;
              htd.textContent = '@@ -' + h.oldStart + ',' + h.oldCount + ' +' + h.newStart + ',' + h.newCount + ' @@';
              hr.append(htd);
              table.append(hr);
              for (const line of h.lines) {
                const tr = document.createElement('tr');
                if (line.type === 'add') tr.className = 'diff-row-add';
                else if (line.type === 'del') tr.className = 'diff-row-del';
                const td1 = document.createElement('td');
                td1.className = 'diff-ln';
                td1.textContent = line.oldLine ?? '';
                const td2 = document.createElement('td');
                td2.className = 'diff-ln';
                td2.textContent = line.newLine ?? '';
                const td3 = document.createElement('td');
                const prefix = line.type === 'add' ? '+' : line.type === 'del' ? '-' : ' ';
                td3.textContent = prefix + line.text;
                tr.append(td1, td2, td3);
                table.append(tr);
              }
            }
            box.append(table);
          }
          diffBody.append(box);
        }
      }

      async function loadDiff(head, base, pathFilter) {
        diffBody.innerHTML = '<p class="muted">差分を読み込み中…</p>';
        const params = new URLSearchParams();
        if (head) params.set('head', head);
        if (base) params.set('base', base);
        if (pathFilter) params.set('path', pathFilter);
        const r = await fetch(apiUrl + '/diff?' + params.toString(), { credentials: 'same-origin' });
        const d = await r.json().catch(() => ({}));
        if (!r.ok) {
          diffBody.textContent = d.error === 'narrow_to_file'
            ? '変更範囲が広いため、対象ファイルの履歴を開いて比較してください。'
            : d.error === 'github_rate_limit'
              ? 'GitHubの読み取り上限に達しました。時間をおいて再度お試しください。'
              : '差分を取得できませんでした: ' + (d.error || r.status);
          return;
        }
        renderDiffResponse(d);
      }

      function syncComparison() {
        const base = diffBase.value.trim();
        const head = diffHead.value.trim();
        const updateUrl = (url) => {
          if (base) url.searchParams.set('compareBase', base);
          else url.searchParams.delete('compareBase');
          if (head && head !== activeRef) url.searchParams.set('compareHead', head);
          else url.searchParams.delete('compareHead');
          return url;
        };
        history.replaceState(null, '', updateUrl(new URL(location.href)));
        document.querySelectorAll('[data-history-page]').forEach((link) => {
          link.href = updateUrl(new URL(link.href));
        });
        const form = document.querySelector('#history-ref-form');
        form.elements.compareBase.value = base;
        form.elements.compareHead.value = head === activeRef ? '' : head;
        const shortRef = (ref) => /^[0-9a-f]{40}$/.test(ref) ? ref.slice(0, 12) : ref;
        document.querySelector('#history-selection').textContent =
          '旧: ' + (shortRef(base) || '新の親') + ' ／ 新: ' + shortRef(head || activeRef);
      }
      diffBase.addEventListener('change', syncComparison);
      diffHead.addEventListener('change', syncComparison);
      syncComparison();

      document.querySelector('#run-diff-btn').addEventListener('click', () => {
        loadDiff(diffHead.value.trim() || activeRef, diffBase.value.trim(), historyPath);
      });
      document.querySelectorAll('.diff-commit-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          const sha = btn.getAttribute('data-commit');
          if (sha) {
            diffHead.value = sha;
            diffBase.value = '';
            syncComparison();
            loadDiff(sha, '', historyPath);
            document.querySelector('#diff-card').scrollIntoView({ behavior: 'smooth' });
          }
        });
      });
      document.querySelectorAll('[data-set-base], [data-set-head]').forEach((btn) => {
        btn.addEventListener('click', () => {
          if (btn.dataset.setBase) diffBase.value = btn.dataset.setBase;
          else diffHead.value = btn.dataset.setHead;
          syncComparison();
        });
      });

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
      document.querySelector('#mint-read')?.addEventListener('click', () => mint('read'));
      document.querySelector('#mint-write')?.addEventListener('click', () => mint('write'));

      document.querySelector('#fork-btn')?.addEventListener('click', async () => {
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

      document.querySelector('#delete-btn')?.addEventListener('click', async () => {
        if (!confirm('リポジトリ「${escape(row.slug)}」を削除しますか？Git 履歴も削除されます。')) return;
        const r = await fetch('/api/repos/' + repoId, {
          method: 'DELETE',
          credentials: 'same-origin',
          headers: { 'x-csrf-token': csrf },
        });
        if (r.status === 204) location.href = '/repos';
        else alert('削除できませんでした');
      });

      document.querySelector('#commit-file')?.addEventListener('submit', async (e) => {
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
              'x-target-branch': activeRef,
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

      const pushStatus = document.querySelector('#push-status');
      document.querySelector('#push-external')?.addEventListener('submit', async (e) => {
        e.preventDefault();
        const f = e.currentTarget;
        pushStatus.textContent = '外部リモートへ Push 中…';
        try {
          const r = await fetch('/api/repos/' + repoId + '/push', {
            method: 'POST',
            credentials: 'same-origin',
            headers: { 'Content-Type': 'application/json', 'x-csrf-token': csrf },
            body: JSON.stringify({
              remoteUrl: f.remoteUrl.value.trim(),
              branch: activeRef,
              remoteBranch: f.remoteBranch.value.trim(),
              token: f.token.value.trim(),
            }),
          });
          const d = await r.json().catch(() => ({}));
          pushStatus.innerHTML = '';
          if (r.ok) {
            const msg = document.createElement('span');
            msg.style.color = 'var(--ok)';
            msg.textContent = d.status === 'up_to_date'
              ? 'すでに最新です (' + String(d.commit || '').slice(0, 7) + ')'
              : 'Push 完了: ' + d.remoteBranch + ' (' + String(d.commit || '').slice(0, 7) + ') ';
            pushStatus.append(msg);
            if (d.compareUrl) {
              const a = document.createElement('a');
              a.className = 'btn btn-primary';
              a.href = d.compareUrl;
              a.target = '_blank';
              a.rel = 'noopener noreferrer';
              a.textContent = 'GitHub で Pull Request を作成';
              pushStatus.append(a);
            }
          } else {
            const err = document.createElement('span');
            err.style.color = 'var(--danger)';
            err.textContent = 'Push に失敗しました: ' + (d.error || r.status);
            pushStatus.append(err);
          }
        } catch {
          pushStatus.textContent = '通信エラーが発生しました。';
        }
      });
    </script>`;

  return reposPage(env, row.slug, html, repoUrl, 200, row.browser);
}

async function openGitHubRepo(request: Request, env: RepoEnv, coordinates: { owner: string; name: string }, operation: 'page' | 'diff' | 'blob', path?: string) {
  const external = new GitHubRepo(coordinates.owner, coordinates.name, MAX_DIFF_FILE_BYTES);
  const info = await external.info();
  const repoUrl = '/repos/github/' + external.name;
  const apiUrl = '/api' + repoUrl;
  if (operation === 'diff') return diffRepoResponse(request, env, external, info.defaultBranch);
  if (operation === 'blob') {
    let filePath: string | null;
    try {
      filePath = normalizeRepoPath(decodeURIComponent(path ?? ''));
    } catch {
      filePath = null;
    }
    if (!filePath) fail(400, 'invalid_path');
    const params = new URL(request.url).searchParams;
    const ref = params.get('ref')?.trim() || info.defaultBranch;
    const commit = /^[0-9a-f]{40}$/.test(ref) ? ref : (await external.readCommit(ref))?.hash;
    if (!commit) fail(404, 'not_found');
    const result = await external.raw(commit, filePath);
    if (!result) fail(404, 'not_found');
    const headers = baseHeaders(env);
    headers.set('Content-Type', (params.get('plain') === '1' || /\.md$/i.test(filePath)) && isTextDiffablePath(filePath)
      ? 'text/plain; charset=utf-8' : contentTypeForPath(filePath));
    return new Response(result.body, { headers });
  }
  return renderRepoDetail(request, env, external, {
    id: external.name, slug: external.name, title: '', description: info.description,
    defaultBranch: info.defaultBranch, branches: new URL(request.url).searchParams.has('source') ? [] : await external.branches(),
    info: { remote: external.remote }, repoUrl, apiUrl, external,
  });
}

export async function handleReposRequest(request: Request, env: RepoEnv, path: string): Promise<Response | null> {
  if (request.method === 'GET' && path === '/repos') return await reposHome(request, env);
  if (request.method === 'GET' && path === '/repos/open') {
    const connection = githubUrlCoordinates(new URL(request.url).searchParams.get('url') ?? '');
    if (!connection) return reposPage(env, 'リポジトリを開く', OPEN_REPO +
      '<p role="alert">公開GitHubリポジトリのHTTPS URLを入力してください。</p>', '/repos', 400);
    return Response.redirect(new URL('/repos/github/' + connection.owner + '/' + connection.name, request.url).toString(), 303);
  }
  if (request.method === 'GET') {
    const externalPage = path.startsWith('/repos/github/') ? githubCoordinates(path.slice('/repos/github'.length)) : null;
    if (externalPage) {
      try {
        return await openGitHubRepo(request, env, externalPage, 'page');
      } catch (error) {
        if (!(error instanceof HttpError) || error.message !== 'github_rate_limit') throw error;
        const remote = 'https://github.com/' + externalPage.owner + '/' + externalPage.name;
        return reposPage(env, 'リポジトリを開く',
          `<div class="card"><div class="card-bd"><p role="alert">GitHubの読み取り上限に達しました。時間をおいて再度お試しください。</p><a class="btn" href="${escape(remote)}">GitHubで開く</a></div></div>`,
          '/repos', 429);
      }
    }
    const api = /^\/api\/repos\/github(\/[^/]+\/[^/]+)\/(diff|blob\/(.+))$/.exec(path);
    const externalApi = api ? githubCoordinates(api[1]) : null;
    if (api && externalApi) return await openGitHubRepo(request, env, externalApi, api[2] === 'diff' ? 'diff' : 'blob', api[3]);
  }

  const pageMatch = REPO_PAGE_PATH.exec(path);
  if (request.method === 'GET' && pageMatch) return await repoDetailPage(request, env, pageMatch[1]);

  if (request.method === 'GET' && path === '/api/repos') return await listRepos(request, env);
  if (request.method === 'POST' && path === '/api/repos') return await createRepo(request, env);
  if (request.method === 'POST' && path === '/api/repos/import') return await importRepo(request, env);

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

  const branchesMatch = REPO_API_BRANCHES.exec(path);
  if (branchesMatch) {
    if (request.method === 'GET') return await listRepoBranches(request, env, branchesMatch[1]);
    if (request.method === 'POST') return await createRepoBranch(request, env, branchesMatch[1]);
  }

  const diffMatch = REPO_API_DIFF.exec(path);
  if (diffMatch && request.method === 'GET') {
    return await diffRepo(request, env, diffMatch[1]);
  }

  const pushMatch = REPO_API_PUSH.exec(path);
  if (pushMatch && request.method === 'POST') {
    return await pushRepoToExternal(request, env, pushMatch[1]);
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
