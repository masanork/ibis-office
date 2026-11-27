// SPDX-License-Identifier: AGPL-3.0-only
// Stateless GitHub App OAuth + Vault repository API for soraneDocs Web.
// Stores the GitHub user access token and selected Vault in an AES-GCM
// encrypted HttpOnly cookie so tokens are never exposed to browser JS.

import { HELP_FILES, normalizeHelpLocale, type HelpFile, type HelpLocale } from './help-content.js';

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

/**
 * Extract `build.content_dir` from a `sorane.yaml` file (defaults to `"content"`
 * when `sorane.yaml` exists without an explicit `build.content_dir`).
 */
export function detectSoraneContentDir(yamlText: string): string {
  let inBuild = false;
  for (const rawLine of yamlText.split(/\r?\n/)) {
    const trimmed = rawLine.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    if (!/^\s/.test(rawLine)) {
      inBuild = /^build\s*:/.test(rawLine);
      continue;
    }
    if (inBuild) {
      const m = /^\s+content_dir\s*:\s*["']?([^"'#\r\n]+?)["']?\s*(?:#.*)?$/.exec(rawLine);
      if (m && m[1]) {
        const sanitized = sanitizeRootPath(m[1]);
        if (sanitized) return sanitized;
      }
    }
  }
  return 'content';
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

const GITHUB_COPILOT_TOKEN_URL = 'https://api.github.com/copilot_internal/v2/token';
const GITHUB_COPILOT_API_BASE = 'https://api.githubcopilot.com';
const COPILOT_EDITOR_VERSION = 'vscode/1.99.0';
const COPILOT_EDITOR_PLUGIN_VERSION = 'copilot-chat/0.26.0';
const COPILOT_INTEGRATION_ID = 'vscode-chat';

export interface AiModelEntry {
  key: string;
  label: string;
  maxInputChars: number;
  vendor?: string;
  fast?: boolean;
  supportsToolCalls?: boolean;
}

interface CopilotTokenResult {
  token: string;
  apiBase: string;
  expiresAt: number;
}

interface CopilotModelRaw {
  id?: string;
  name?: string;
  vendor?: string;
  model_picker_enabled?: boolean;
  policy?: { state?: string };
  capabilities?: {
    type?: string;
    limits?: {
      max_prompt_tokens?: number;
      max_context_window_tokens?: number;
    };
    supports?: {
      streaming?: boolean;
      tool_calls?: boolean;
    };
  };
}

function copilotHeaders(token: string, extra?: Record<string, string>): Record<string, string> {
  return {
    Authorization: `Bearer ${token}`,
    Accept: 'application/json',
    'User-Agent': 'soraneDocs-Web',
    'Editor-Version': COPILOT_EDITOR_VERSION,
    'Editor-Plugin-Version': COPILOT_EDITOR_PLUGIN_VERSION,
    'Copilot-Integration-Id': COPILOT_INTEGRATION_ID,
    ...extra,
  };
}

function isFastModelId(id: string): boolean {
  const lower = id.toLowerCase();
  return (
    lower.includes('mini') ||
    lower.includes('flash') ||
    lower.includes('haiku') ||
    lower.includes('nano') ||
    lower.includes('lite') ||
    lower.includes('small')
  );
}

export function normalizeCopilotModels(payload: unknown): {
  models: AiModelEntry[];
  defaultModel: string | null;
  completionModel: string | null;
} {
  const rawList: CopilotModelRaw[] = Array.isArray(payload)
    ? payload
    : payload && typeof payload === 'object' && Array.isArray((payload as { data?: unknown }).data)
      ? ((payload as { data: CopilotModelRaw[] }).data)
      : [];

  const seen = new Set<string>();
  const allChat: Array<AiModelEntry & { pickerEnabled: boolean; streaming: boolean }> = [];

  for (const item of rawList) {
    const id = typeof item?.id === 'string' ? item.id.trim() : '';
    if (!id || seen.has(id)) continue;
    seen.add(id);

    const policyState = item.policy?.state?.toLowerCase();
    if (policyState === 'disabled' || policyState === 'unconfigured' || policyState === 'blocked') {
      continue;
    }
    const kind = item.capabilities?.type?.toLowerCase();
    if (kind && kind !== 'chat') continue;

    const streaming = item.capabilities?.supports?.streaming !== false;
    const supportsToolCalls = Boolean(item.capabilities?.supports?.tool_calls);
    const tokens =
      item.capabilities?.limits?.max_prompt_tokens ??
      item.capabilities?.limits?.max_context_window_tokens ??
      16000;
    const maxInputChars = Math.max(8000, Math.min(120000, Math.round(tokens * 2.5)));
    const rawName = typeof item.name === 'string' ? item.name.trim() : '';
    const label = rawName && rawName.toLowerCase() !== id.toLowerCase()
      ? `GitHub Copilot: ${rawName} (${id})`
      : `GitHub Copilot: ${id}`;
    const vendor = typeof item.vendor === 'string' && item.vendor.trim() ? item.vendor.trim() : undefined;

    allChat.push({
      key: id,
      label,
      maxInputChars,
      ...(vendor ? { vendor } : {}),
      fast: isFastModelId(id),
      supportsToolCalls,
      pickerEnabled: item.model_picker_enabled !== false,
      streaming,
    });
  }

  const pickerFiltered = allChat.filter(m => m.pickerEnabled);
  const chosen = pickerFiltered.length > 0 ? pickerFiltered : allChat;
  const models: AiModelEntry[] = chosen.map(({ pickerEnabled: _p, streaming: _s, ...entry }) => entry);

  const streamingChosen = chosen.filter(m => m.streaming);
  const pool = streamingChosen.length > 0 ? streamingChosen : chosen;
  const defaultEntry =
    pool.find(m => m.supportsToolCalls && !m.fast) ??
    pool.find(m => !m.fast) ??
    pool[0] ??
    null;
  const completionEntry =
    pool.find(m => m.fast) ??
    pool[0] ??
    null;

  return {
    models,
    defaultModel: defaultEntry?.key ?? null,
    completionModel: completionEntry?.key ?? null,
  };
}

async function exchangeCopilotToken(
  githubAccessToken: string,
): Promise<{ ok: true; data: CopilotTokenResult } | { ok: false; status: number; code: string; detail?: string }> {
  let res: Response;
  try {
    res = await fetch(GITHUB_COPILOT_TOKEN_URL, {
      method: 'GET',
      headers: {
        ...copilotHeaders(githubAccessToken),
        'X-GitHub-Api-Version': '2022-11-28',
      },
    });
  } catch (err) {
    return {
      ok: false,
      status: 502,
      code: 'ai_failed',
      detail: err instanceof Error ? err.message : 'copilot_token_fetch_failed',
    };
  }

  if (!res.ok) {
    const errBody = (await res.json().catch(() => ({}))) as { message?: string };
    const code =
      res.status === 401
        ? 'login_required'
        : res.status === 403 || res.status === 404
          ? 'copilot_unavailable'
          : res.status === 429
            ? 'rate_limited'
            : 'ai_failed';
    return { ok: false, status: res.status, code, detail: errBody.message };
  }

  const parsed = (await res.json().catch(() => null)) as {
    token?: string;
    expires_at?: number;
    endpoints?: { api?: string };
  } | null;
  const token = typeof parsed?.token === 'string' ? parsed.token.trim() : '';
  if (!token) {
    return { ok: false, status: 502, code: 'copilot_unavailable', detail: 'missing copilot session token' };
  }
  const rawApiBase = typeof parsed?.endpoints?.api === 'string' ? parsed.endpoints.api.trim().replace(/\/+$/, '') : '';
  const apiBase = rawApiBase.startsWith('https://') ? rawApiBase : GITHUB_COPILOT_API_BASE;
  return {
    ok: true,
    data: {
      token,
      apiBase,
      expiresAt: typeof parsed?.expires_at === 'number' ? parsed.expires_at : 0,
    },
  };
}

async function fetchCopilotModelCatalog(
  copilot: CopilotTokenResult,
): Promise<
  | { ok: true; models: AiModelEntry[]; defaultModel: string | null; completionModel: string | null }
  | { ok: false; status: number; code: string; detail?: string }
> {
  let res: Response;
  try {
    res = await fetch(`${copilot.apiBase}/models`, {
      method: 'GET',
      headers: copilotHeaders(copilot.token),
    });
  } catch (err) {
    return {
      ok: false,
      status: 502,
      code: 'ai_failed',
      detail: err instanceof Error ? err.message : 'copilot_models_fetch_failed',
    };
  }

  if (!res.ok) {
    const errBody = (await res.json().catch(() => ({}))) as { error?: { message?: string }; message?: string };
    const code =
      res.status === 401
        ? 'login_required'
        : res.status === 403 || res.status === 404
          ? 'copilot_unavailable'
          : res.status === 429
            ? 'rate_limited'
            : 'ai_failed';
    return {
      ok: false,
      status: res.status,
      code,
      detail: errBody.error?.message || errBody.message,
    };
  }

  const data = await res.json().catch(() => null);
  const catalog = normalizeCopilotModels(data);
  if (catalog.models.length === 0) {
    return { ok: false, status: 502, code: 'copilot_unavailable', detail: 'no_chat_models_available' };
  }
  return { ok: true, ...catalog };
}

async function deriveCsrfToken(session: GitHubSessionPayload, secret: string): Promise<string> {
  const data = new TextEncoder().encode(`sorane-ai-csrf:${secret}:${session.login}:${session.iat}`);
  const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', data));
  return toBase64Url(hash.slice(0, 24));
}

export function isGitHubVaultRoute(pathname: string): boolean {
  return (
    pathname.startsWith('/auth/github/') ||
    pathname.startsWith('/api/github/') ||
    pathname.startsWith('/api/ai/')
  );
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
    let installed = false;
    let canCreateRepo = false;
    let repoSelection: string | null = null;
    let installSettingsUrl: string | null = null;
    const instRes = await githubFetch('/user/installations?per_page=50', session.accessToken);
    if (instRes.status === 401) {
      return json(
        { error: 'GitHub の認証トークンが期限切れです。再ログインしてください。', reauthRequired: true },
        401,
        { 'Set-Cookie': `${SESSION_COOKIE}=; Max-Age=0; ${cookieFlags(request)}` },
      );
    }
    if (instRes.ok) {
      const instData = (await instRes.json()) as {
        installations?: Array<{
          id: number;
          account?: { login?: string };
          repository_selection?: string;
          html_url?: string;
          permissions?: Record<string, string>;
        }>;
      };
      const installations = instData.installations ?? [];
      installed = installations.length > 0;
      const personalInst =
        installations.find(i => i.account?.login?.toLowerCase() === session.login.toLowerCase()) ??
        installations[0];
      if (personalInst) {
        repoSelection = personalInst.repository_selection ?? null;
        installSettingsUrl = personalInst.html_url ?? null;
        const hasCreatePerm =
          personalInst.permissions?.repository_creation === 'write' ||
          personalInst.permissions?.administration === 'write';
        canCreateRepo = hasCreatePerm && personalInst.repository_selection !== 'selected';
      }
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
      if (userReposRes.status === 401) {
        return json(
          { error: 'GitHub の認証トークンが期限切れです。再ログインしてください。', reauthRequired: true },
          401,
          { 'Set-Cookie': `${SESSION_COOKIE}=; Max-Age=0; ${cookieFlags(request)}` },
        );
      }
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
    return json({ repos, installUrl, installed, canCreateRepo, repoSelection, installSettingsUrl });
  }

  function buildInitialIndexMarkdown(repo: string, locale: HelpLocale): string {
    if (locale === 'en') {
      return [
        '---',
        'type: index',
        `title: ${repo} — soraneDocs Guide`,
        'description: Personal knowledge and documents managed with soraneDocs.',
        'profile: sorane-okf/0.1',
        '---',
        '',
        `# ${repo}`,
        '',
        'Welcome to your private document repository created with soraneDocs.',
        'This repository is your home for storing, editing, and committing notes, documents, spreadsheets, and slides directly from your browser or desktop.',
        '',
        '## Help & Syntax Reference',
        '',
        'All Markdown syntax guides and ibisOffice / soraneDocs instructions are included in the `help/` folder:',
        '',
        '- [[help/index|Help Index]]',
        '- [[help/getting-started|Getting Started]]',
        '- [[help/shortcuts|Keyboard Shortcuts]]',
        '- [[help/markdown-basics|Markdown Basics & Extensions]]',
        '- [[help/marp-slides|Marp Slide Syntax]]',
        '- [[help/workbook-markdown|Workbook Markdown Syntax]]',
        '- [[help/mermaid-diagrams|Mermaid Diagrams Guide]]',
        '- [[help/okf-and-vault|OKF & Vault Workflow]]',
        '- [[help/office-and-lawtext|Office Documents, Lawtext & Structured Data]]',
        '',
        '## 1. Quick Start',
        '',
        '1. **Open a document**: Click any file from the repository list on the start view.',
        '2. **Edit and save**: When you save a repository file, it is committed directly to this repository after confirmation.',
        '3. **Switch repository or subfolder**: Use the `GitHub: ...` button at the top of the start view.',
        '',
        '## 2. Supported Formats',
        '',
        '| Category | Extensions | Typical Use |',
        '| --- | --- | --- |',
        '| Markdown & Notes | `.md`, `.markdown` | Notes, meeting minutes, specs, OKF knowledge |',
        '| Office Documents | `.docx`, `.jtd`, `.xdw` | Proposals, contracts, Ichitaro / DocuWorks |',
        '| Spreadsheets & Data | `.xlsx`, `.csv`, `.tsv`, `.json` | Budgets, ledgers, structured data |',
        '| Slides & Diagrams | `.pptx`, `.vsdx` | Presentations, architecture diagrams |',
        '',
        '## 3. Suggested Folder Layout',
        '',
        '```text',
        `${repo}/`,
        '├── index.md      # Repository guide & table of contents',
        '├── help/         # Usage guides & Markdown syntax reference',
        '├── notes/        # Daily notes & meeting minutes (Markdown)',
        '├── docs/         # Proposals, specs & handouts (DOCX / XLSX / PPTX)',
        '└── archive/      # Reference materials',
        '```',
        '',
      ].join('\n');
    }

    return [
      '---',
      'type: index',
      `title: ${repo} — soraneDocs ガイド`,
      'description: Personal knowledge and documents managed with soraneDocs.',
      'profile: sorane-okf/0.1',
      '---',
      '',
      `# ${repo}`,
      '',
      'soraneDocs で作成したプライベートドキュメントリポジトリへようこそ。',
      'このリポジトリは、あなたの文書・メモ・表計算・スライドを安全に保管し、ブラウザやデスクトップから直接編集・保存するための拠点です。',
      '',
      '## ヘルプ・記法ガイド',
      '',
      'すべての Markdown 記法や ibisOffice / soraneDocs の操作方法は `help/` フォルダにまとまっています。',
      '',
      '- [[help/index|ヘルプ目次]]',
      '- [[help/getting-started|基本操作ガイド]]',
      '- [[help/shortcuts|キーボードショートカット一覧]]',
      '- [[help/markdown-basics|Markdown 基本・拡張記法]]',
      '- [[help/marp-slides|Marp スライド記法]]',
      '- [[help/workbook-markdown|Workbook Markdown 記法]]',
      '- [[help/mermaid-diagrams|Mermaid 図表ガイド]]',
      '- [[help/okf-and-vault|OKF と Vault 活用]]',
      '- [[help/office-and-lawtext|Office 文書・Lawtext・構造化データ]]',
      '',
      '## 1. 基本的な使い方',
      '',
      '1. **文書を開く**: スタート画面のリポジトリ一覧からファイルをクリックして開きます。',
      '2. **編集して保存する**: リポジトリから開いたファイルを保存・ダウンロードすると、確認ダイアログのあとにこのリポジトリへ直接コミットされます。',
      '3. **リポジトリやフォルダを切り替える**: スタート画面上部の `GitHub: ...` ボタンから、別のリポジトリやサブフォルダ（例: `docs`）を指定できます。',
      '',
      '## 2. 対応しているファイル形式',
      '',
      '| カテゴリ | 対応拡張子 | 主な用途 |',
      '| --- | --- | --- |',
      '| マークダウン・文書 | `.md`, `.markdown` | メモ、議事録、設計書、OKF ナレッジ |',
      '| オフィス文書 | `.docx`, `.jtd`, `.xdw` | 企画書、契約書、一太郎・DocuWorks 文書 |',
      '| 表計算・データ | `.xlsx`, `.csv`, `.tsv`, `.json` | 集計表、台帳、データ一覧 |',
      '| プレゼン・図面 | `.pptx`, `.vsdx` | 発表資料、構成図、フロー図 |',
      '',
      '## 3. おすすめのフォルダ構成',
      '',
      '```text',
      `${repo}/`,
      '├── index.md      # このガイド・全体の目次',
      '├── help/         # 操作ガイド・Markdown 記法リファレンス',
      '├── notes/        # 日々のメモ・議事録（Markdown）',
      '├── docs/         # 企画書・仕様書・配布資料（DOCX / XLSX / PPTX）',
      '└── archive/      # 保管用の参照資料',
      '```',
      '',
      '## 4. OKF メモの書き方ヒント',
      '',
      'Markdown ファイルの先頭に YAML フロントマター（`---` で囲んだメタデータ）を記述すると、文書の種別やタグを構造化して管理できます。',
      '',
      '```markdown',
      '---',
      'type: note',
      'title: 会議メモ',
      'tags: [meeting, project]',
      '---',
      '',
      '# 会議メモ',
      '```',
      '',
      '## 5. トラブルシューティング',
      '',
      '- **新しいリポジトリが見えない／保存できない場合**: GitHub App（soraneDocs）のインストール設定で、リポジトリアクセスが **All repositories** になっているか、対象リポジトリが許可されているか確認してください。',
      '- **ログインが切れた場合**: スタート画面上部の GitHub ボタンから再ログインすると、最新のトークンで再接続できます。',
      '',
    ].join('\n');
  }

  async function commitFilesToBranch(
    owner: string,
    repo: string,
    branch: string,
    accessToken: string,
    files: HelpFile[],
    message: string,
  ): Promise<{ ok: true; commitSha: string } | { ok: false; status: number; error: string }> {
    const refRes = await githubFetch(
      `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/git/ref/heads/${encodeURIComponent(branch)}`,
      accessToken,
    );
    if (!refRes.ok) {
      return { ok: false, status: refRes.status, error: 'ブランチの参照を取得できませんでした。' };
    }
    const refData = (await refRes.json()) as { object?: { sha?: string } };
    const headCommitSha = refData.object?.sha;
    if (!headCommitSha) {
      return { ok: false, status: 502, error: 'ブランチの最新コミット SHA を取得できませんでした。' };
    }

    const commitRes = await githubFetch(
      `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/git/commits/${headCommitSha}`,
      accessToken,
    );
    if (!commitRes.ok) {
      return { ok: false, status: commitRes.status, error: '最新コミット情報を取得できませんでした。' };
    }
    const commitData = (await commitRes.json()) as { tree?: { sha?: string } };
    const baseTreeSha = commitData.tree?.sha;
    if (!baseTreeSha) {
      return { ok: false, status: 502, error: 'ベースツリー SHA を取得できませんでした。' };
    }

    const treeRes = await githubFetch(
      `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/git/trees`,
      accessToken,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          base_tree: baseTreeSha,
          tree: files.map(f => ({
            path: f.path,
            mode: '100644',
            type: 'blob',
            content: f.content,
          })),
        }),
      },
    );
    if (!treeRes.ok) {
      return { ok: false, status: treeRes.status, error: 'Git ツリーの作成に失敗しました。' };
    }
    const treeData = (await treeRes.json()) as { sha?: string };
    if (!treeData.sha) {
      return { ok: false, status: 502, error: '作成した Git ツリーの SHA が空です。' };
    }

    const newCommitRes = await githubFetch(
      `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/git/commits`,
      accessToken,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          message,
          tree: treeData.sha,
          parents: [headCommitSha],
        }),
      },
    );
    if (!newCommitRes.ok) {
      return { ok: false, status: newCommitRes.status, error: 'コミットの作成に失敗しました。' };
    }
    const newCommitData = (await newCommitRes.json()) as { sha?: string };
    if (!newCommitData.sha) {
      return { ok: false, status: 502, error: '作成したコミットの SHA が空です。' };
    }

    const updateRefRes = await githubFetch(
      `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/git/refs/heads/${encodeURIComponent(branch)}`,
      accessToken,
      {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ sha: newCommitData.sha }),
      },
    );
    if (!updateRefRes.ok) {
      return { ok: false, status: updateRefRes.status, error: 'ブランチ参照の更新に失敗しました。' };
    }

    return { ok: true, commitSha: newCommitData.sha };
  }

  if (pathname === '/api/github/repos/create' && request.method === 'POST') {
    const body = (await request.json().catch(() => null)) as { name?: unknown; description?: unknown; lang?: unknown } | null;
    const rawName = typeof body?.name === 'string' && body.name.trim() ? body.name.trim() : 'soraneDocs';
    if (!SAFE_SEGMENT_RE.test(rawName)) {
      return json({ error: 'リポジトリ名には英数字・ハイフン・アンダースコア・ドットのみ使用できます。' }, 400);
    }
    const locale = normalizeHelpLocale(
      typeof body?.lang === 'string' ? body.lang : request.headers.get('Accept-Language'),
    );
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
      if (createRes.status === 401) {
        return json(
          { error: 'GitHub の認証トークンが期限切れです。再ログインしてください。', reauthRequired: true },
          401,
          { 'Set-Cookie': `${SESSION_COOKIE}=; Max-Age=0; ${cookieFlags(request)}` },
        );
      }
      const fallbackCreateUrl = `https://github.com/new?name=${encodeURIComponent(rawName)}&visibility=private`;
      if (createRes.status === 403 || createRes.status === 404) {
        const acceptedPermissions = createRes.headers.get('X-Accepted-GitHub-Permissions')?.trim() || null;
        let installation: {
          account: string | null;
          repositorySelection: string | null;
          permissions: Record<string, string>;
        } | null = null;
        let installSettingsUrl: string | null = null;
        try {
          const instRes = await githubFetch('/user/installations?per_page=20', session.accessToken);
          if (instRes.ok) {
            const instData = (await instRes.json()) as {
              installations?: Array<{
                account?: { login?: string };
                repository_selection?: string;
                html_url?: string;
                permissions?: Record<string, string>;
              }>;
            };
            const list = instData.installations ?? [];
            const matched = list.find(i => i.account?.login?.toLowerCase() === session.login.toLowerCase()) ?? list[0];
            if (matched) {
              installSettingsUrl = matched.html_url ?? null;
              installation = {
                account: matched.account?.login ?? null,
                repositorySelection: matched.repository_selection ?? null,
                permissions: matched.permissions ?? {},
              };
            }
          }
        } catch {
          /* Best-effort diagnostics only. */
        }

        const diagParts: string[] = [];
        if (createData.message) diagParts.push(createData.message);
        if (acceptedPermissions) diagParts.push(`required: ${acceptedPermissions}`);
        if (installation) {
          if (installation.repositorySelection) diagParts.push(`selection: ${installation.repositorySelection}`);
          const granted = Object.entries(installation.permissions)
            .map(([k, v]) => `${k}:${v}`)
            .sort()
            .join(', ');
          diagParts.push(`granted: ${granted || 'none'}`);
        } else {
          diagParts.push('installation: none');
        }
        const detail = diagParts.length ? ` [${diagParts.join(' / ')}]` : '';
        return json(
          {
            error: `GitHub App でリポジトリを自動作成できませんでした${detail}。アプリ権限（Repository creation: Write）の承認と「All repositories」設定を確認するか、リンク先から GitHub で作成してください。`,
            fallbackCreateUrl,
            diagnostics: { acceptedPermissions, installation },
            ...(installSettingsUrl ? { installSettingsUrl } : {}),
            ...(installUrl ? { installUrl } : {}),
          },
          403,
        );
      }
      if (createRes.status === 422) {
        let installSettingsUrl: string | null = null;
        try {
          const instRes = await githubFetch('/user/installations?per_page=20', session.accessToken);
          if (instRes.ok) {
            const instData = (await instRes.json()) as {
              installations?: Array<{ account?: { login?: string }; html_url?: string }>;
            };
            const list = instData.installations ?? [];
            const matched = list.find(i => i.account?.login?.toLowerCase() === session.login.toLowerCase()) ?? list[0];
            installSettingsUrl = matched?.html_url ?? null;
          }
        } catch {
          /* Best-effort link lookup. */
        }
        return json(
          {
            error: `リポジトリ ${session.login}/${rawName} はすでに存在するため新規作成できません。既存リポジトリ一覧から選択するか、一覧に表示されない場合は GitHub App のアクセス許可を確認してください。`,
            alreadyExists: true,
            fallbackCreateUrl,
            ...(installSettingsUrl ? { installSettingsUrl } : {}),
            ...(installUrl ? { installUrl } : {}),
          },
          422,
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
    const repoUrl = createData.html_url || `https://github.com/${owner}/${repo}`;

    // Initialize with an OKF-compliant, structured help index.md so the repository is useful immediately.
    const initialIndex = buildInitialIndexMarkdown(repo, locale);
    const initPayload = JSON.stringify({
      message: 'Initialize soraneDocs repository with OKF help index.md',
      content: toBase64(new TextEncoder().encode(initialIndex)),
      branch,
    });

    let initRes: Response | null = null;
    const retryDelaysMs = [300, 700];
    for (let attempt = 0; attempt <= retryDelaysMs.length; attempt++) {
      initRes = await githubFetch(
        `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/contents/index.md`,
        session.accessToken,
        {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: initPayload,
        },
      );
      if (initRes.ok) break;
      if (initRes.status !== 404 && initRes.status !== 502 && initRes.status !== 503) break;
      if (attempt < retryDelaysMs.length) {
        await new Promise(resolve => setTimeout(resolve, retryDelaysMs[attempt]));
      }
    }

    if (!initRes || !initRes.ok) {
      const initErr = (await initRes?.json().catch(() => ({}))) as { message?: string };
      let installSettingsUrl: string | null = null;
      try {
        const instRes = await githubFetch('/user/installations?per_page=20', session.accessToken);
        if (instRes.ok) {
          const instData = (await instRes.json()) as {
            installations?: Array<{ account?: { login?: string }; html_url?: string }>;
          };
          const list = instData.installations ?? [];
          const matched = list.find(i => i.account?.login?.toLowerCase() === session.login.toLowerCase()) ?? list[0];
          installSettingsUrl = matched?.html_url ?? null;
        }
      } catch {
        /* Best-effort link lookup. */
      }
      const status = initRes?.status ?? 502;
      const reason = initErr?.message ? ` (${initErr.message})` : '';
      return json(
        {
          error: `リポジトリ ${owner}/${repo} は作成されましたが、初期ファイル index.md の書き込みに失敗しました${reason}。GitHub App のリポジトリアクセス設定が「Only select repositories」の場合は「All repositories」に変更するか、作成したリポジトリを許可リストに追加してから選択してください。`,
          initFailed: true,
          repoUrl,
          ...(installSettingsUrl ? { installSettingsUrl } : {}),
          ...(installUrl ? { installUrl } : {}),
        },
        status === 403 || status === 404 ? 409 : status,
      );
    }

    // Best-effort commit of the localized help/ directory in a single commit.
    const helpCommit = await commitFilesToBranch(
      owner,
      repo,
      branch,
      session.accessToken,
      HELP_FILES[locale],
      `Add soraneDocs help files (${locale})`,
    );

    const nextVault: VaultSelection = { owner, repo, branch, rootPath: '' };
    const sealed = await sealSession({ ...session, vault: nextVault }, env.GITHUB_SESSION_SECRET!);
    return json(
      { ok: true, vault: nextVault, repoUrl, helpSynced: helpCommit.ok },
      201,
      { 'Set-Cookie': `${SESSION_COOKIE}=${sealed}; Max-Age=${SESSION_MAX_AGE}; ${cookieFlags(request)}` },
    );
  }

  if (pathname === '/api/github/sync-help' && request.method === 'POST') {
    if (!session.vault) {
      return json({ error: 'No Vault repository selected.' }, 400);
    }
    const body = (await request.json().catch(() => null)) as { lang?: unknown } | null;
    const locale = normalizeHelpLocale(
      typeof body?.lang === 'string' ? body.lang : request.headers.get('Accept-Language'),
    );
    const { owner, repo, branch, rootPath } = session.vault;
    const prefix = rootPath ? `${rootPath.replace(/\/+$/, '')}/` : '';
    const files: HelpFile[] = HELP_FILES[locale].map(f => ({
      path: `${prefix}${f.path}`,
      content: f.content,
    }));

    const result = await commitFilesToBranch(
      owner,
      repo,
      branch,
      session.accessToken,
      files,
      `Update soraneDocs help files (${locale})`,
    );
    if (!result.ok) {
      return json({ error: result.error }, result.status);
    }
    return json({ ok: true, locale, count: files.length, commitSha: result.commitSha });
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
    let rootPath = sanitizeRootPath(body.rootPath);

    const checkRes = await githubFetch(`/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`, session.accessToken);
    if (!checkRes.ok) {
      return json({ error: 'Repository not accessible with current GitHub login.' }, checkRes.status === 404 ? 404 : 403);
    }
    const repoMeta = (await checkRes.json()) as { default_branch?: string };
    if (!branch) branch = repoMeta.default_branch || 'main';

    let detectedSoraneContentDir: string | null = null;
    if (!rootPath && body.rootPath === undefined) {
      try {
        const cfgRes = await githubFetch(
          `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/contents/sorane.yaml?ref=${encodeURIComponent(branch)}`,
          session.accessToken,
        );
        if (cfgRes.ok) {
          const cfgData = (await cfgRes.json()) as { content?: string; encoding?: string };
          if (cfgData.encoding === 'base64' && typeof cfgData.content === 'string') {
            const cleanedB64 = cfgData.content.replace(/\s+/g, '');
            const bin = atob(cleanedB64);
            const bytes = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            const yamlText = new TextDecoder('utf-8').decode(bytes);
            detectedSoraneContentDir = detectSoraneContentDir(yamlText);
            rootPath = detectedSoraneContentDir;
          }
        }
      } catch {
        /* Best-effort sorane.yaml detection; keep empty rootPath on failure. */
      }
    }

    const nextVault: VaultSelection = { owner, repo, branch, rootPath };
    const sealed = await sealSession({ ...session, vault: nextVault }, env.GITHUB_SESSION_SECRET!);
    return json(
      { ok: true, vault: nextVault, ...(detectedSoraneContentDir ? { detectedSoraneContentDir } : {}) },
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

  if (pathname === '/api/ai/models' && request.method === 'GET') {
    const csrf = await deriveCsrfToken(session, env.GITHUB_SESSION_SECRET!);
    const exchanged = await exchangeCopilotToken(session.accessToken);
    if (!exchanged.ok) {
      return json({
        csrf,
        provider: 'github-copilot',
        models: [],
        defaultModel: null,
        completionModel: null,
        error: exchanged.code,
        ...(exchanged.detail ? { detail: exchanged.detail } : {}),
      });
    }
    const catalog = await fetchCopilotModelCatalog(exchanged.data);
    if (!catalog.ok) {
      return json({
        csrf,
        provider: 'github-copilot',
        models: [],
        defaultModel: null,
        completionModel: null,
        error: catalog.code,
        ...(catalog.detail ? { detail: catalog.detail } : {}),
      });
    }
    return json({
      csrf,
      provider: 'github-copilot',
      models: catalog.models,
      defaultModel: catalog.defaultModel,
      completionModel: catalog.completionModel,
    });
  }

  if (pathname === '/api/ai/chat' && request.method === 'POST') {
    const expectedCsrf = await deriveCsrfToken(session, env.GITHUB_SESSION_SECRET!);
    const suppliedCsrf = request.headers.get('X-Sorane-CSRF')?.trim() || '';
    if (!suppliedCsrf || suppliedCsrf !== expectedCsrf) {
      return json({ error: 'invalid_csrf' }, 403);
    }

    const body = (await request.json().catch(() => null)) as {
      model?: unknown;
      messages?: unknown;
      purpose?: unknown;
    } | null;
    if (!body || !Array.isArray(body.messages) || body.messages.length === 0 || body.messages.length > 24) {
      return json({ error: 'invalid_message' }, 400);
    }

    const exchanged = await exchangeCopilotToken(session.accessToken);
    if (!exchanged.ok) {
      return json(
        { error: exchanged.code, ...(exchanged.detail ? { detail: exchanged.detail } : {}) },
        exchanged.status === 401 ? 401 : 502,
      );
    }

    const catalog = await fetchCopilotModelCatalog(exchanged.data);
    if (!catalog.ok) {
      return json(
        { error: catalog.code, ...(catalog.detail ? { detail: catalog.detail } : {}) },
        catalog.status === 401 ? 401 : 502,
      );
    }

    const requestedModel = typeof body.model === 'string' ? body.model.trim() : '';
    const preferFast = body.purpose === 'completion';
    const fallbackKey = preferFast
      ? (catalog.completionModel || catalog.defaultModel)
      : (catalog.defaultModel || catalog.completionModel);
    const allowedModel =
      (requestedModel ? catalog.models.find(m => m.key === requestedModel) : undefined) ??
      (fallbackKey ? catalog.models.find(m => m.key === fallbackKey) : undefined) ??
      catalog.models[0];
    if (!allowedModel) {
      return json({ error: 'copilot_unavailable' }, 502);
    }

    const totalChars = body.messages.reduce<number>((sum, item) => {
      if (item && typeof item === 'object' && 'content' in item && typeof item.content === 'string') {
        return sum + item.content.length;
      }
      return sum;
    }, 0);
    if (totalChars > allowedModel.maxInputChars) {
      return json({ error: 'too_long' }, 413);
    }

    let upstream: Response;
    try {
      upstream = await fetch(`${exchanged.data.apiBase}/chat/completions`, {
        method: 'POST',
        headers: copilotHeaders(exchanged.data.token, {
          Accept: 'text/event-stream',
          'Content-Type': 'application/json',
          'Openai-Intent': 'conversation-panel',
        }),
        body: JSON.stringify({
          model: allowedModel.key,
          messages: body.messages,
          stream: true,
        }),
      });
    } catch (err) {
      return json(
        { error: 'ai_failed', detail: err instanceof Error ? err.message : 'copilot_chat_fetch_failed' },
        502,
      );
    }

    if (!upstream.ok) {
      const code = upstream.status === 401
        ? 'login_required'
        : upstream.status === 403 || upstream.status === 404
          ? 'copilot_unavailable'
          : upstream.status === 429
            ? 'rate_limited'
            : upstream.status === 413
              ? 'too_long'
              : 'ai_failed';
      return json({ error: code }, upstream.status === 401 ? 401 : 502);
    }

    return new Response(upstream.body, {
      status: 200,
      headers: {
        'Content-Type': 'text/event-stream; charset=utf-8',
        'Cache-Control': 'no-store',
        'X-Sorane-Model': allowedModel.key,
      },
    });
  }

  return json({ error: 'Not found.' }, 404);
}
