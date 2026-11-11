import {
  createRemoteJWKSet,
  decodeProtectedHeader,
  errors,
  importJWK,
  jwtVerify,
  SignJWT,
} from 'jose';

// Minimal capabilities consumed by the shared protocol library. Worker Env types
// remain generated from each application's Wrangler configuration.
type SessionContext = { DB: D1Database; ISSUER: string; RP_ORIGIN: string; CLIENT_ID: string } & {
  RP_PRIVATE_JWK: string;
  LOCAL_ONLY?: string;
  DEMO_ONLY?: string;
};
type Session = {
  token_hash: string;
  sid: string;
  sub: string;
  auth_time: number;
  lease_until: number;
  parent_expires_at: number;
  idle_expires_at: number;
  idle_timeout_seconds: number;
};
const BROWSER = '__Host-help-browser';
const SESSION = '__Host-help-session';
const ASSERTION_TYPE = 'urn:ietf:params:oauth:client-assertion-type:jwt-bearer';
const jwksByIssuer = new Map<string, ReturnType<typeof createRemoteJWKSet>>();
function issuerKeys(issuer: string): ReturnType<typeof createRemoteJWKSet> {
  let keys = jwksByIssuer.get(issuer);
  if (!keys) {
    keys = createRemoteJWKSet(new URL(`${issuer}/jwks`), { timeoutDuration: 5000 });
    jwksByIssuer.set(issuer, keys);
  }
  return keys;
}
const now = () => Math.floor(Date.now() / 1000);
const random = () => crypto.randomUUID() + crypto.randomUUID();
const b64 = (bytes: Uint8Array) =>
  btoa(String.fromCodePoint(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replaceAll('=', '');
const hash = async (value: string) =>
  b64(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value))));
const escape = (value: unknown) =>
  String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
const cookieHeader = (name: string, value: string, maxAge: number) =>
  `${name}=${value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=${maxAge}`;

class HttpError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}
function fail(status: number, message: string): never {
  throw new HttpError(status, message);
}
function cookie(request: Request, name: string): string {
  const matches = (request.headers.get('cookie') ?? '')
    .split(';')
    .map((part) => part.trim())
    .filter((part) => part.startsWith(`${name}=`));
  if (matches.length > 1) fail(400, 'duplicate_cookie');
  return matches[0]?.slice(name.length + 1) ?? '';
}
function baseHeaders(env: SessionContext): Headers {
  return new Headers({
    'Cache-Control': 'no-store',
    'Referrer-Policy': 'strict-origin',
    'X-Robots-Tag': env.DEMO_ONLY === 'true' ? 'noindex, nofollow' : 'noindex',
    'X-Content-Type-Options': 'nosniff',
    'Content-Security-Policy': `default-src 'none'; style-src 'unsafe-inline'; form-action 'self' ${env.ISSUER}; frame-ancestors 'none'; base-uri 'none'`,
  });
}
function redirect(env: SessionContext, path: string, setCookie?: string): Response {
  const headers = baseHeaders(env);
  headers.set('Location', path.startsWith('http') ? path : `${env.RP_ORIGIN}${path}`);
  if (setCookie) headers.set('Set-Cookie', setCookie);
  return new Response(null, { status: 303, headers });
}
function sameOrigin(request: Request, env: SessionContext): void {
  if (request.headers.get('origin') !== env.RP_ORIGIN) fail(403, 'invalid_origin');
}
async function readForm(request: Request, env: SessionContext): Promise<URLSearchParams> {
  sameOrigin(request, env);
  if (request.headers.get('content-type')?.split(';')[0] !== 'application/x-www-form-urlencoded')
    fail(415, 'unsupported_media_type');
  const reader = request.body?.getReader();
  if (!reader) fail(400, 'missing_body');
  let size = 0;
  const chunks: Uint8Array[] = [];
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > 8192) {
      await reader.cancel();
      fail(413, 'request_too_large');
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  const form = new URLSearchParams(
    new TextDecoder('utf-8', { fatal: true, ignoreBOM: false }).decode(bytes),
  );
  if ([...form].length > 12 || new Set([...form.keys()]).size !== [...form].length)
    fail(400, 'invalid_form');
  const csrf = await hash(cookie(request, BROWSER));
  if (!cookie(request, BROWSER) || form.get('csrf') !== csrf) fail(403, 'invalid_csrf');
  return form;
}
async function assertion(env: SessionContext, audience: string): Promise<string> {
  const jwk = JSON.parse(env.RP_PRIVATE_JWK) as JsonWebKey & { kid?: string };
  if (jwk.kty !== 'EC' || jwk.crv !== 'P-256' || !jwk.kid) fail(503, 'rp_key_unavailable');
  const issued = now();
  return new SignJWT({ sub: env.CLIENT_ID, jti: random() })
    .setProtectedHeader({ alg: 'ES256', typ: 'JWT', kid: jwk.kid })
    .setIssuer(env.CLIENT_ID)
    .setAudience(audience)
    .setIssuedAt(issued)
    .setExpirationTime(issued + 60)
    .sign(await importJWK(jwk, 'ES256'));
}
async function opPost(
  env: SessionContext,
  path: string,
  values: Record<string, string>,
): Promise<Record<string, unknown>> {
  const endpoint = `${env.ISSUER}${path}`;
  const data = {
    ...values,
    client_id: env.CLIENT_ID,
    client_assertion_type: ASSERTION_TYPE,
    client_assertion: await assertion(env, endpoint),
  };
  // The disposable local OP uses a form for this endpoint; the product OP uses JSON.
  const jsonBody = path === '/session/check' && env.LOCAL_ONLY !== 'true';
  const response = await fetch(endpoint, {
    method: 'POST',
    headers: {
      'Content-Type': jsonBody ? 'application/json' : 'application/x-www-form-urlencoded',
    },
    body: jsonBody ? JSON.stringify(data) : new URLSearchParams(data),
    redirect: 'manual',
    signal: AbortSignal.timeout(5000),
  });
  if (!response.ok) fail(503, 'issuer_unavailable');
  const reader = response.body?.getReader();
  if (!reader) fail(503, 'issuer_response_missing');
  let size = 0;
  const chunks: Uint8Array[] = [];
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > 16384) {
      await reader.cancel();
      fail(503, 'issuer_response_too_large');
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  const text = new TextDecoder().decode(bytes);
  return JSON.parse(text) as Record<string, unknown>;
}
async function checkSession(env: SessionContext, sid: string, sub: string, authTime: number) {
  const started = now();
  const result = await opPost(env, '/session/check', { sid });
  if (
    result.active !== true ||
    result.sub !== sub ||
    result.auth_time !== authTime ||
    typeof result.expires_at !== 'number' ||
    typeof result.lease_ttl !== 'number' ||
    typeof result.app_idle_timeout !== 'number' ||
    !Number.isSafeInteger(result.expires_at) ||
    !Number.isSafeInteger(result.lease_ttl) ||
    !Number.isSafeInteger(result.app_idle_timeout) ||
    result.lease_ttl <= 0 ||
    result.lease_ttl > 300 ||
    result.app_idle_timeout <= 0
  )
    fail(401, 'session_inactive');
  const lease = Math.min(started + result.lease_ttl, result.expires_at);
  if (lease <= now()) fail(401, 'session_expired');
  return {
    lease,
    parent: result.expires_at,
    idle: Math.min(now() + result.app_idle_timeout, result.expires_at),
    idleTimeout: result.app_idle_timeout,
  };
}
async function current(
  request: Request,
  env: SessionContext,
  forceCheck = false,
): Promise<Session | null> {
  const token = cookie(request, SESSION);
  if (!token) return null;
  const db = env.DB.withSession('first-primary');
  let session = await db
    .prepare(
      'SELECT * FROM rp_session WHERE token_hash=? AND idle_expires_at>? AND parent_expires_at>?',
    )
    .bind(await hash(token), now(), now())
    .first<Session>();
  if (!session) return null;
  let valid: Awaited<ReturnType<typeof checkSession>> | null = null;
  if (forceCheck || session.lease_until <= now()) {
    try {
      valid = await checkSession(env, session.sid, session.sub, session.auth_time);
    } catch (error) {
      // A known revocation must also invalidate a still-unexpired cached lease.
      if (error instanceof HttpError && error.status === 401)
        await db
          .prepare('DELETE FROM rp_session WHERE token_hash=?')
          .bind(session.token_hash)
          .run();
      throw error;
    }
  }
  const timestamp = now();
  if (valid && valid.lease <= timestamp) fail(401, 'session_expired');
  const parent = Math.min(session.parent_expires_at, valid?.parent ?? session.parent_expires_at);
  session = await db
    .prepare(
      'UPDATE rp_session SET lease_until=MIN(MAX(lease_until,?),?), parent_expires_at=MIN(parent_expires_at,?), idle_timeout_seconds=?, idle_expires_at=MIN(?+?,parent_expires_at,?) WHERE token_hash=? AND idle_expires_at>? AND parent_expires_at>? AND (lease_until>? OR ?=1) RETURNING *',
    )
    .bind(
      valid?.lease ?? session.lease_until,
      parent,
      parent,
      valid?.idleTimeout ?? session.idle_timeout_seconds,
      timestamp,
      valid?.idleTimeout ?? session.idle_timeout_seconds,
      parent,
      session.token_hash,
      timestamp,
      timestamp,
      timestamp,
      valid ? 1 : 0,
    )
    .first<Session>();
  if (!session) fail(401, 'session_changed');
  return session;
}
async function requireSession(
  request: Request,
  env: SessionContext,
  forceCheck = false,
): Promise<Session> {
  const session = await current(request, env, forceCheck);
  if (!session) fail(401, 'login_required');
  return session;
}
function formCsrf(value: string): string {
  return `<input type="hidden" name="csrf" value="${escape(value)}">`;
}
async function browserCsrf(request: Request): Promise<string> {
  return hash(cookie(request, BROWSER));
}
async function login(request: Request, env: SessionContext, locale = 'ja'): Promise<Response> {
  await readForm(request, env);
  const browser = cookie(request, BROWSER);
  const state = random(),
    nonce = random(),
    verifier = random();
  await env.DB.prepare(
    'INSERT INTO login_transaction(state_hash,browser_hash,nonce,verifier,expires_at) VALUES(?,?,?,?,?)',
  )
    .bind(await hash(state), await hash(browser), nonce, verifier, now() + 300)
    .run();
  const target = new URL(`${env.ISSUER}/authorize`);
  target.search = new URLSearchParams({
    client_id: env.CLIENT_ID,
    redirect_uri: `${env.RP_ORIGIN}/callback`,
    response_type: 'code',
    scope: 'openid',
    state,
    nonce,
    code_challenge: await hash(verifier),
    code_challenge_method: 'S256',
    ui_locales: locale,
  }).toString();
  return redirect(env, target.href);
}
async function callback(
  request: Request,
  env: SessionContext,
  url: URL,
  destination = '/tickets',
): Promise<Response> {
  if ([...url.searchParams].length !== new Set([...url.searchParams.keys()]).size)
    fail(400, 'invalid_callback');
  const state = url.searchParams.get('state'),
    code = url.searchParams.get('code');
  if (!state || !code || url.searchParams.get('iss') !== env.ISSUER) fail(400, 'invalid_callback');
  const transaction = await env.DB.prepare(
    'DELETE FROM login_transaction WHERE state_hash=? AND browser_hash=? AND expires_at>? RETURNING nonce,verifier',
  )
    .bind(await hash(state), await hash(cookie(request, BROWSER)), now())
    .first<{ nonce: string; verifier: string }>();
  if (!transaction) fail(400, 'invalid_transaction');
  const token = await opPost(env, '/token', {
    grant_type: 'authorization_code',
    code,
    redirect_uri: `${env.RP_ORIGIN}/callback`,
    code_verifier: transaction.verifier,
  });
  if (typeof token.id_token !== 'string' || token.id_token.length > 16384)
    fail(401, 'invalid_id_token');
  const header = decodeProtectedHeader(token.id_token);
  if (
    header.alg !== 'ES256' ||
    header.typ !== 'JWT' ||
    !header.kid ||
    header.jku ||
    header.jwk ||
    header.x5u
  )
    fail(401, 'invalid_id_token');
  const { payload } = await jwtVerify(
    token.id_token,
    createRemoteJWKSet(new URL(`${env.ISSUER}/jwks`)),
    {
      issuer: env.ISSUER,
      audience: env.CLIENT_ID,
      algorithms: ['ES256'],
      typ: 'JWT',
      clockTolerance: 60,
      requiredClaims: ['iat', 'exp', 'iss', 'aud', 'sub'],
    },
  );
  if (
    payload.nonce !== transaction.nonce ||
    payload.aud !== env.CLIENT_ID ||
    typeof payload.sub !== 'string' ||
    typeof payload.sid !== 'string' ||
    typeof payload.auth_time !== 'number' ||
    !Number.isSafeInteger(payload.auth_time) ||
    !payload.iat ||
    !payload.exp ||
    payload.exp - payload.iat > 600 ||
    payload.auth_time > payload.iat ||
    (payload.azp && payload.azp !== env.CLIENT_ID)
  )
    fail(401, 'invalid_id_token');
  const valid = await checkSession(env, payload.sid, payload.sub, payload.auth_time);
  if (env.DEMO_ONLY === 'true') {
    valid.parent = Math.min(valid.parent, now() + 3600);
    valid.lease = Math.min(valid.lease, valid.parent);
    valid.idle = Math.min(valid.idle, valid.parent);
  }
  const secret = random();
  const inserted = await env.DB.prepare(
    'INSERT INTO rp_session(token_hash,sid,sub,auth_time,lease_until,parent_expires_at,idle_expires_at,idle_timeout_seconds) SELECT ?,?,?,?,?,?,?,? WHERE NOT EXISTS(SELECT 1 FROM logout_tombstone WHERE sid=? AND expires_at>?)',
  )
    .bind(
      await hash(secret),
      payload.sid,
      payload.sub,
      payload.auth_time,
      valid.lease,
      valid.parent,
      valid.idle,
      valid.idleTimeout,
      payload.sid,
      now(),
    )
    .run();
  if (inserted.meta.changes !== 1) fail(401, 'session_revoked');
  return redirect(env, destination, cookieHeader(SESSION, secret, valid.parent - now()));
}

async function readLogoutToken(request: Request): Promise<string> {
  if (request.headers.get('content-type')?.split(';')[0] !== 'application/x-www-form-urlencoded')
    fail(415, 'unsupported_media_type');
  const reader = request.body?.getReader();
  if (!reader) fail(400, 'missing_body');
  let size = 0;
  const chunks: Uint8Array[] = [];
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > 16384) {
      await reader.cancel();
      fail(413, 'request_too_large');
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  let params: URLSearchParams;
  try {
    params = new URLSearchParams(
      new TextDecoder('utf-8', { fatal: true, ignoreBOM: false }).decode(bytes),
    );
  } catch {
    fail(400, 'invalid_form');
  }
  const tokens = params.getAll('logout_token');
  if (tokens.length !== 1 || tokens[0].length > 12000) fail(400, 'invalid_logout_token');
  return tokens[0];
}

async function verifiedLogoutPayload(token: string, env: SessionContext) {
  let header;
  try {
    header = decodeProtectedHeader(token);
  } catch {
    fail(400, 'invalid_logout_token');
  }
  if (
    header.alg !== 'ES256' ||
    header.typ !== 'logout+jwt' ||
    !header.kid ||
    header.jku ||
    header.jwk ||
    header.x5u
  )
    fail(400, 'invalid_logout_token');
  let payload;
  try {
    ({ payload } = await jwtVerify(token, issuerKeys(env.ISSUER), {
      issuer: env.ISSUER,
      audience: env.CLIENT_ID,
      algorithms: ['ES256'],
      typ: 'logout+jwt',
      clockTolerance: 60,
      requiredClaims: ['iss', 'aud', 'iat', 'exp', 'jti'],
    }));
  } catch (error) {
    if (
      error instanceof TypeError ||
      error instanceof errors.JWKSTimeout ||
      (error instanceof errors.JOSEError && error.constructor === errors.JOSEError)
    )
      fail(503, 'issuer_keys_unavailable');
    fail(400, 'invalid_logout_token');
  }
  return payload;
}

async function backchannel(request: Request, env: SessionContext): Promise<Response> {
  const token = await readLogoutToken(request);
  const payload = await verifiedLogoutPayload(token, env);
  const event = payload.events;
  const marker =
    event &&
    typeof event === 'object' &&
    !Array.isArray(event) &&
    'http://schemas.openid.net/event/backchannel-logout' in event // NOSONAR: OIDC event identifier, not a network URL.
      ? event['http://schemas.openid.net/event/backchannel-logout']
      : null;
  if (
    typeof payload.sid !== 'string' ||
    !payload.sid ||
    payload.sid.length > 128 ||
    (payload.sub !== undefined && (typeof payload.sub !== 'string' || !payload.sub)) ||
    typeof payload.jti !== 'string' ||
    !payload.jti ||
    typeof payload.iat !== 'number' ||
    typeof payload.exp !== 'number' ||
    payload.exp <= payload.iat ||
    payload.exp - payload.iat > 300 ||
    payload.iat > now() + 60 ||
    Object.hasOwn(payload, 'nonce') ||
    !marker ||
    typeof marker !== 'object' ||
    Array.isArray(marker) ||
    Object.keys(marker).length !== 0
  )
    fail(400, 'invalid_logout_token');
  const existing = await env.DB.withSession('first-primary')
    .prepare('SELECT sub FROM rp_session WHERE sid=? LIMIT 1')
    .bind(payload.sid)
    .first<{ sub: string }>();
  if (existing && payload.sub !== undefined && existing.sub !== payload.sub)
    fail(400, 'invalid_logout_token');
  await env.DB.batch([
    env.DB.prepare(
      'INSERT INTO logout_tombstone(sid,expires_at) VALUES(?,?) ON CONFLICT(sid) DO UPDATE SET expires_at=MAX(expires_at,excluded.expires_at)',
    ).bind(payload.sid, now() + 32 * 86400),
    env.DB.prepare('DELETE FROM rp_session WHERE sid=?').bind(payload.sid),
  ]);
  return new Response(null, { status: 200, headers: baseHeaders(env) });
}

export async function logout(request: Request, env: SessionContext): Promise<Response> {
  await readForm(request, env);
  const token = cookie(request, SESSION);
  if (token)
    await env.DB.prepare('DELETE FROM rp_session WHERE token_hash=?')
      .bind(await hash(token))
      .run();
  return redirect(env, '/', cookieHeader(SESSION, '', 0));
}
export async function cleanup(env: SessionContext): Promise<void> {
  const expired = now();
  await env.DB.batch([
    env.DB.prepare('DELETE FROM login_transaction WHERE expires_at<?').bind(expired),
    env.DB.prepare('DELETE FROM logout_tombstone WHERE expires_at<?').bind(expired),
    env.DB.prepare('DELETE FROM rp_session WHERE idle_expires_at<? OR parent_expires_at<?').bind(
      expired,
      expired,
    ),
  ]);
}
export {
  BROWSER,
  SESSION,
  now,
  random,
  hash,
  escape,
  cookieHeader,
  HttpError,
  fail,
  cookie,
  baseHeaders,
  redirect,
  readForm,
  current,
  requireSession,
  formCsrf,
  browserCsrf,
  login,
  callback,
  backchannel,
};
export type { Session };
