// SPDX-License-Identifier: AGPL-3.0-only
// Stateless GitHub App OAuth + Vault repository API for soraneDocs Web.
// Stores the GitHub user access token and selected Vault in an AES-GCM
// encrypted HttpOnly cookie so tokens are never exposed to browser JS.

export interface GitHubVaultEnv {
  GITHUB_APP_CLIENT_ID?: string;
  GITHUB_APP_CLIENT_SECRET?: string;
  GITHUB_APP_SLUG?: string;
  GITHUB_SESSION_SECRET?: string;
}

export interface VaultSelection {
  owner: string;
  repo: string;
  branch: string;
  rootPath: string;
}

export interface GitHubSessionPayload {
  accessToken: string;
  login: string;
  vault: VaultSelection | null;
  iat: number;
}

const SESSION_COOKIE = 'sorane_gh_session';
const STATE_COOKIE = 'sorane_gh_state';
const SESSION_MAX_AGE = 60 * 60 * 24 * 7; // 7 days
const GITHUB_API = 'https://api.github.com';
const SUPPORTED_EXT_RE = /\.(?:md|markdown|docx|xlsx|pptx|csv|tsv|jtd|vsdx|xdw|json)$/i;
const SAFE_SEGMENT_RE = /^[A-Za-z0-9._-]{1,100}$/;
const SAFE_BRANCH_RE = /^[A-Za-z0-9._/-]{1,120}$/;

function json(data: unknown, status = 200, extraHeaders?: Record<string, string>): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      'Content-Type': 'application/json; charset=utf-8',
      'Cache-Control': 'no-store',
      ...extraHeaders,
    },
  });
}

function parseCookies(header: string | null): Record<string, string> {
  const out: Record<string, string> = {};
  if (!header) return out;
  for (const part of header.split(';')) {
    const idx = part.indexOf('=');
    if (idx <= 0) continue;
    const k = part.slice(0, idx).trim();
    const v = part.slice(idx + 1).trim();
    if (k) out[k] = v;
  }
  return out;
}

function toBase64Url(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function fromBase64Url(str: string): Uint8Array {
  const padded = str.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (str.length % 4)) % 4);
  const binary = atob(padded);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

function toBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

async function deriveKey(secret: string): Promise<CryptoKey> {
  const hash = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(secret));
  return crypto.subtle.importKey('raw', hash, { name: 'AES-GCM' }, false, ['encrypt', 'decrypt']);
}

export async function sealSession(payload: GitHubSessionPayload, secret: string): Promise<string> {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const key = await deriveKey(secret);
  const plaintext = new TextEncoder().encode(JSON.stringify(payload));
  const ciphertext = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, plaintext));
  const combined = new Uint8Array(iv.length + ciphertext.length);
  combined.set(iv, 0);
  combined.set(ciphertext, iv.length);
  return toBase64Url(combined);
}

export async function unsealSession(token: string, secret: string): Promise<GitHubSessionPayload | null> {
  try {
    const raw = fromBase64Url(token);
    if (raw.length < 13) return null;
    const iv = raw.slice(0, 12);
    const ciphertext = raw.slice(12);
    const key = await deriveKey(secret);
    const plaintext = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, key, ciphertext);
    const parsed = JSON.parse(new TextDecoder().decode(plaintext)) as Partial<GitHubSessionPayload>;
    if (typeof parsed.accessToken !== 'string' || typeof parsed.login !== 'string') return null;
    return {
      accessToken: parsed.accessToken,
      login: parsed.login,
      vault: parsed.vault ?? null,
      iat: typeof parsed.iat === 'number' ? parsed.iat : 0,
    };
  } catch {
    return null;
  }
}

function isConfigured(env: GitHubVaultEnv): boolean {
  return Boolean(env.GITHUB_APP_CLIENT_ID && env.GITHUB_APP_CLIENT_SECRET && env.GITHUB_SESSION_SECRET);
}

function cookieFlags(request: Request): string {
  const secure = new URL(request.url).protocol === 'https:' ? '; Secure' : '';
  return `HttpOnly; SameSite=Lax; Path=/${secure}`;
}

function sanitizeRootPath(raw: unknown): string {
  if (typeof raw !== 'string') return '';
  const cleaned = raw.trim().replace(/^\/+|\/+$/g, '');
  if (!cleaned) return '';
  const parts = cleaned.split('/').filter(Boolean);
  if (parts.some(p => p === '.' || p === '..' || p.includes('\\'))) return '';
  return parts.join('/');
}

function sanitizeRelativeFilePath(raw: string | null): string | null {
  if (!raw) return null;
  const cleaned = raw.trim().replace(/^\/+/, '');
  if (!cleaned || cleaned.length > 300 || cleaned.includes('\\')) return null;
  const parts = cleaned.split('/');
  if (parts.some(p => !p || p === '.' || p === '..')) return null;
  return parts.join('/');
}

function encodeGitHubPath(path: string): string {
  return path.split('/').map(encodeURIComponent).join('/');
}

async function githubFetch(path: string, accessToken: string, init?: RequestInit): Promise<Response> {
  const headers = new Headers(init?.headers);
  headers.set('Authorization', `Bearer ${accessToken}`);
  headers.set('Accept', 'application/vnd.github+json');
  headers.set('User-Agent', 'soraneDocs-Web');
  headers.set('X-GitHub-Api-Version', '2022-11-28');
  return fetch(`${GITHUB_API}${path}`, { ...init, headers });
}

export function isGitHubVaultRoute(pathname: string): boolean {
  return pathname.startsWith('/auth/github/') || pathname.startsWith('/api/github/');
}

export async function dispatchGitHubVault(request: Request, env: GitHubVaultEnv): Promise<Response | null> {
  const url = new URL(request.url);
  const { pathname } = url;
  if (!isGitHubVaultRoute(pathname)) return null;

  const configured = isConfigured(env);
  const installUrl = env.GITHUB_APP_SLUG
    ? `https://github.com/apps/${encodeURIComponent(env.GITHUB_APP_SLUG)}/installations/new`
    : null;

  if (pathname === '/api/github/session' && request.method === 'GET') {
    if (!configured) {
      return json({ configured: false, loggedIn: false, login: null, vault: null, installUrl });
    }
    const cookies = parseCookies(request.headers.get('Cookie'));
    const session = cookies[SESSION_COOKIE]
      ? await unsealSession(cookies[SESSION_COOKIE], env.GITHUB_SESSION_SECRET!)
      : null;
    if (!session) {
      return json({ configured: true, loggedIn: false, login: null, vault: null, installUrl });
    }
    return json({
      configured: true,
      loggedIn: true,
      login: session.login,
      vault: session.vault,
      installUrl,
    });
  }

  if (pathname === '/auth/github/logout' && request.method === 'POST') {
    return json(
      { ok: true },
      200,
      { 'Set-Cookie': `${SESSION_COOKIE}=; Max-Age=0; ${cookieFlags(request)}` },
    );
  }

  if (!configured) {
    return json(
      {
        error: 'GitHub App is not configured on this deployment yet.',
        hint: 'Set GITHUB_APP_CLIENT_ID, GITHUB_APP_CLIENT_SECRET, and GITHUB_SESSION_SECRET via wrangler secret put.',
      },
      503,
    );
  }

  if (pathname === '/auth/github/login' && request.method === 'GET') {
    const stateBytes = crypto.getRandomValues(new Uint8Array(24));
    const state = toBase64Url(stateBytes);
    const redirectUri = `${url.origin}/auth/github/callback`;
    const authUrl = new URL('https://github.com/login/oauth/authorize');
    authUrl.searchParams.set('client_id', env.GITHUB_APP_CLIENT_ID!);
    authUrl.searchParams.set('redirect_uri', redirectUri);
    authUrl.searchParams.set('state', state);
    return new Response(null, {
      status: 302,
      headers: {
        Location: authUrl.toString(),
        'Cache-Control': 'no-store',
        'Set-Cookie': `${STATE_COOKIE}=${state}; Max-Age=600; ${cookieFlags(request)}`,
      },
    });
  }

  if (pathname === '/auth/github/callback' && request.method === 'GET') {
    const code = url.searchParams.get('code');
    const state = url.searchParams.get('state');
    const cookies = parseCookies(request.headers.get('Cookie'));
    if (!code || !state || !cookies[STATE_COOKIE] || cookies[STATE_COOKIE] !== state) {
      return json({ error: 'Invalid or expired OAuth state.' }, 400);
    }

    const tokenRes = await fetch('https://github.com/login/oauth/access_token', {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
        'User-Agent': 'soraneDocs-Web',
      },
      body: JSON.stringify({
        client_id: env.GITHUB_APP_CLIENT_ID!,
        client_secret: env.GITHUB_APP_CLIENT_SECRET!,
        code,
        redirect_uri: `${url.origin}/auth/github/callback`,
      }),
    });
    if (!tokenRes.ok) {
      return json({ error: 'GitHub token exchange failed.' }, 502);
    }
    const tokenData = (await tokenRes.json()) as { access_token?: string; error?: string };
    if (!tokenData.access_token) {
      return json({ error: tokenData.error || 'Missing GitHub access token.' }, 400);
    }

    const userRes = await githubFetch('/user', tokenData.access_token);
    if (!userRes.ok) {
      return json({ error: 'Failed to load GitHub user profile.' }, 502);
    }
    const userData = (await userRes.json()) as { login?: string };
    if (!userData.login) {
      return json({ error: 'GitHub user profile missing login.' }, 502);
    }

    // Preserve previous vault selection if re-authenticating as the same user.
    const existing = cookies[SESSION_COOKIE]
      ? await unsealSession(cookies[SESSION_COOKIE], env.GITHUB_SESSION_SECRET!)
      : null;
    const vault = existing && existing.login === userData.login ? existing.vault : null;
    const sealed = await sealSession(
      {
        accessToken: tokenData.access_token,
        login: userData.login,
        vault,
        iat: Date.now(),
      },
      env.GITHUB_SESSION_SECRET!,
    );

    const headers = new Headers({
      Location: `${url.origin}/?github=connected`,
      'Cache-Control': 'no-store',
    });
    headers.append('Set-Cookie', `${SESSION_COOKIE}=${sealed}; Max-Age=${SESSION_MAX_AGE}; ${cookieFlags(request)}`);
    headers.append('Set-Cookie', `${STATE_COOKIE}=; Max-Age=0; ${cookieFlags(request)}`);
    return new Response(null, { status: 302, headers });
  }

  // All remaining /api/github/* endpoints require a valid session cookie.
  const cookies = parseCookies(request.headers.get('Cookie'));
  const session = cookies[SESSION_COOKIE]
    ? await unsealSession(cookies[SESSION_COOKIE], env.GITHUB_SESSION_SECRET!)
    : null;
  if (!session) {
    return json({ error: 'Authentication required.' }, 401);
  }

  if (pathname === '/api/github/repos' && request.method === 'GET') {
    const collected = new Map<string, {
      owner: string;
      repo: string;
      fullName: string;
      defaultBranch: string;
      private: boolean;
      updatedAt: string;
    }>();

    // 1. Prefer GitHub App installation repositories (fine-grained user selection).
    const instRes = await githubFetch('/user/installations?per_page=50', session.accessToken);
    if (instRes.ok) {
      const instData = (await instRes.json()) as { installations?: Array<{ id: number }> };
      const installations = instData.installations ?? [];
      for (const inst of installations.slice(0, 10)) {
        const repoRes = await githubFetch(
          `/user/installations/${inst.id}/repositories?per_page=100`,
          session.accessToken,
        );
        if (!repoRes.ok) continue;
        const repoData = (await repoRes.json()) as {
          repositories?: Array<{
            full_name: string;
            name: string;
            owner: { login: string };
            default_branch?: string;
            private?: boolean;
            updated_at?: string;
          }>;
        };
        for (const r of repoData.repositories ?? []) {
          collected.set(r.full_name, {
            owner: r.owner.login,
            repo: r.name,
            fullName: r.full_name,
            defaultBranch: r.default_branch || 'main',
            private: Boolean(r.private),
            updatedAt: r.updated_at || '',
          });
        }
      }
    }

    // 2. Fallback to /user/repos if installation list is empty (e.g., OAuth App mode).
    if (collected.size === 0) {
      const userReposRes = await githubFetch('/user/repos?sort=updated&per_page=50', session.accessToken);
      if (userReposRes.ok) {
        const list = (await userReposRes.json()) as Array<{
          full_name: string;
          name: string;
          owner: { login: string };
          default_branch?: string;
          private?: boolean;
          updated_at?: string;
        }>;
        for (const r of list) {
          collected.set(r.full_name, {
            owner: r.owner.login,
            repo: r.name,
            fullName: r.full_name,
            defaultBranch: r.default_branch || 'main',
            private: Boolean(r.private),
            updatedAt: r.updated_at || '',
          });
        }
      }
    }

    const repos = [...collected.values()].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    return json({ repos, installUrl });
  }

  if (pathname === '/api/github/repos/create' && request.method === 'POST') {
    const body = (await request.json().catch(() => null)) as { name?: unknown; description?: unknown } | null;
    const rawName = typeof body?.name === 'string' && body.name.trim() ? body.name.trim() : 'soraneDocs';
    if (!SAFE_SEGMENT_RE.test(rawName)) {
      return json({ error: 'リポジトリ名には英数字・ハイフン・アンダースコア・ドットのみ使用できます。' }, 400);
    }
    const description = typeof body?.description === 'string' && body.description.trim()
      ? body.description.trim().slice(0, 200)
      : 'Personal documents repository for soraneDocs';

    const createRes = await githubFetch('/user/repos', session.accessToken, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name: rawName,
        description,
        private: true,
        auto_init: false,
      }),
    });
    const createData = (await createRes.json().catch(() => ({}))) as {
      name?: string;
      owner?: { login?: string };
      default_branch?: string;
      html_url?: string;
      message?: string;
    };
    if (!createRes.ok) {
      const fallbackCreateUrl = `https://github.com/new?name=${encodeURIComponent(rawName)}&visibility=private`;
      if (createRes.status === 403 || createRes.status === 404) {
        return json(
          {
            error: 'GitHub App にリポジトリ作成権限（Administration: Read and write）がないため自動作成できませんでした。下のリンクから GitHub で作成してください。',
            fallbackCreateUrl,
          },
          403,
        );
      }
      return json(
        {
          error: createData.message || `リポジトリ作成に失敗しました (HTTP ${createRes.status})`,
          fallbackCreateUrl,
        },
        createRes.status,
      );
    }

    const owner = createData.owner?.login || session.login;
    const repo = createData.name || rawName;
    const branch = createData.default_branch || 'main';

    // Initialize with an OKF-compliant index.md so the repository is ready to open immediately.
    const initialIndex = [
      '---',
      'type: index',
      `title: ${repo}`,
      'description: Personal knowledge and documents managed with soraneDocs.',
      'profile: sorane-okf/0.1',
      '---',
      '',
      `# ${repo}`,
      '',
      'soraneDocs で作成したプライベートドキュメントリポジトリです。',
      '',
    ].join('\n');
    await githubFetch(
      `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/contents/index.md`,
      session.accessToken,
      {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          message: 'Initialize soraneDocs repository with OKF index.md',
          content: toBase64(new TextEncoder().encode(initialIndex)),
          branch,
        }),
      },
    );

    const nextVault: VaultSelection = { owner, repo, branch, rootPath: '' };
    const sealed = await sealSession({ ...session, vault: nextVault }, env.GITHUB_SESSION_SECRET!);
    return json(
      { ok: true, vault: nextVault, repoUrl: createData.html_url || `https://github.com/${owner}/${repo}` },
      201,
      { 'Set-Cookie': `${SESSION_COOKIE}=${sealed}; Max-Age=${SESSION_MAX_AGE}; ${cookieFlags(request)}` },
    );
  }

  if (pathname === '/api/github/vault' && request.method === 'POST') {
    const body = (await request.json().catch(() => null)) as Partial<VaultSelection> | null;
    if (!body || typeof body.owner !== 'string' || typeof body.repo !== 'string') {
      return json({ error: 'owner and repo are required.' }, 400);
    }
    const owner = body.owner.trim();
    const repo = body.repo.trim();
    if (!SAFE_SEGMENT_RE.test(owner) || !SAFE_SEGMENT_RE.test(repo)) {
      return json({ error: 'Invalid repository owner or name.' }, 400);
    }
    let branch = typeof body.branch === 'string' && body.branch.trim() ? body.branch.trim() : '';
    if (branch && !SAFE_BRANCH_RE.test(branch)) {
      return json({ error: 'Invalid branch name.' }, 400);
    }
    const rootPath = sanitizeRootPath(body.rootPath);

    const checkRes = await githubFetch(`/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`, session.accessToken);
    if (!checkRes.ok) {
      return json({ error: 'Repository not accessible with current GitHub login.' }, checkRes.status === 404 ? 404 : 403);
    }
    const repoMeta = (await checkRes.json()) as { default_branch?: string };
    if (!branch) branch = repoMeta.default_branch || 'main';

    const nextVault: VaultSelection = { owner, repo, branch, rootPath };
    const sealed = await sealSession({ ...session, vault: nextVault }, env.GITHUB_SESSION_SECRET!);
    return json(
      { ok: true, vault: nextVault },
      200,
      { 'Set-Cookie': `${SESSION_COOKIE}=${sealed}; Max-Age=${SESSION_MAX_AGE}; ${cookieFlags(request)}` },
    );
  }

  if (pathname === '/api/github/tree' && request.method === 'GET') {
    if (!session.vault) {
      return json({ error: 'No Vault repository selected.' }, 400);
    }
    const { owner, repo, branch, rootPath } = session.vault;
    const treeRes = await githubFetch(
      `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/git/trees/${encodeURIComponent(branch)}?recursive=1`,
      session.accessToken,
    );
    if (!treeRes.ok) {
      return json({ error: 'Failed to list repository files.' }, treeRes.status);
    }
    const treeData = (await treeRes.json()) as {
      truncated?: boolean;
      tree?: Array<{ path: string; type: string; size?: number; sha?: string }>;
    };
    const prefix = rootPath ? `${rootPath}/` : '';
    const files = (treeData.tree ?? [])
      .filter(entry => entry.type === 'blob' && (!prefix || entry.path.startsWith(prefix)) && SUPPORTED_EXT_RE.test(entry.path))
      .slice(0, 100)
      .map(entry => ({
        path: entry.path,
        name: entry.path.split('/').pop() || entry.path,
        size: entry.size ?? 0,
        sha: entry.sha ?? '',
      }));
    return json({ vault: session.vault, truncated: Boolean(treeData.truncated), files });
  }

  if (pathname === '/api/github/file' && request.method === 'GET') {
    if (!session.vault) return json({ error: 'No Vault repository selected.' }, 400);
    const filePath = sanitizeRelativeFilePath(url.searchParams.get('path'));
    if (!filePath) return json({ error: 'Invalid path parameter.' }, 400);
    const { owner, repo, branch } = session.vault;
    const metaRes = await githubFetch(
      `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/contents/${encodeGitHubPath(filePath)}?ref=${encodeURIComponent(branch)}`,
      session.accessToken,
    );
    if (!metaRes.ok) {
      return json({ error: 'Failed to read file from GitHub.' }, metaRes.status);
    }
    const meta = (await metaRes.json()) as { sha?: string; content?: string; encoding?: string; download_url?: string };
    let bytes: Uint8Array<ArrayBuffer>;
    if (meta.encoding === 'base64' && typeof meta.content === 'string') {
      const clean = meta.content.replace(/\s+/g, '');
      const binary = atob(clean);
      bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    } else if (meta.sha) {
      const blobRes = await githubFetch(
        `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/git/blobs/${encodeURIComponent(meta.sha)}`,
        session.accessToken,
        { headers: { Accept: 'application/vnd.github.raw+json' } },
      );
      if (!blobRes.ok) return json({ error: 'Failed to read large blob from GitHub.' }, blobRes.status);
      bytes = new Uint8Array(await blobRes.arrayBuffer());
    } else {
      return json({ error: 'Unsupported GitHub content response.' }, 502);
    }

    const headers = new Headers({
      'Content-Type': 'application/octet-stream',
      'Cache-Control': 'no-store',
    });
    if (meta.sha) headers.set('ETag', `"${meta.sha}"`);
    return new Response(bytes, { status: 200, headers });
  }

  if (pathname === '/api/github/file' && request.method === 'PUT') {
    if (!session.vault) return json({ error: 'No Vault repository selected.' }, 400);
    const filePath = sanitizeRelativeFilePath(url.searchParams.get('path'));
    if (!filePath) return json({ error: 'Invalid path parameter.' }, 400);
    const message = (request.headers.get('X-Commit-Message') || `Update ${filePath} via soraneDocs`).slice(0, 200);
    const ifMatch = request.headers.get('If-Match')?.replace(/^"+|"+$/g, '').trim() || undefined;
    const bodyBytes = new Uint8Array(await request.arrayBuffer());
    const { owner, repo, branch } = session.vault;

    let sha = ifMatch;
    if (!sha) {
      const existingRes = await githubFetch(
        `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/contents/${encodeGitHubPath(filePath)}?ref=${encodeURIComponent(branch)}`,
        session.accessToken,
      );
      if (existingRes.ok) {
        const existing = (await existingRes.json()) as { sha?: string };
        sha = existing.sha;
      }
    }

    const putRes = await githubFetch(
      `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/contents/${encodeGitHubPath(filePath)}`,
      session.accessToken,
      {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          message,
          content: toBase64(bodyBytes),
          branch,
          ...(sha ? { sha } : {}),
        }),
      },
    );
    const putData = (await putRes.json().catch(() => ({}))) as {
      content?: { sha?: string; path?: string };
      commit?: { sha?: string; html_url?: string };
      message?: string;
    };
    if (!putRes.ok) {
      return json({ error: putData.message || 'Failed to commit file to GitHub.' }, putRes.status);
    }
    return json({
      ok: true,
      sha: putData.content?.sha ?? null,
      commitSha: putData.commit?.sha ?? null,
      commitUrl: putData.commit?.html_url ?? null,
    });
  }

  return json({ error: 'Not found.' }, 404);
}
