// SPDX-License-Identifier: AGPL-3.0-only
/**
 * Phase 4C: OID4VP `response_mode=direct_post` CORS Relay Worker
 *
 * Security invariants:
 * - Only `POST /api/vc/direct-post` with JSON `{ response_uri, form_body }` is accepted.
 * - Strict SSRF protection: only `https://` URLs with public hostnames/IPs are allowed.
 *   Loopback, private IPv4/IPv6 ranges, link-local / cloud metadata (`169.254.169.254`),
 *   CGNAT (`100.64.0.0/10`), `.local` / `.internal` hostnames, and userinfo (`@`) are rejected.
 * - `vp_token` / `form_body` contents are NEVER logged or persisted.
 */

const DIRECT_POST_PATH = '/api/vc/direct-post';
const MAX_URI_LEN = 2048;
const MAX_FORM_BODY_BYTES = 512 * 1024; // 512 KiB

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      'content-type': 'application/json; charset=utf-8',
      'cache-control': 'no-store',
    },
  });
}

function isPrivateOrReservedIpv4(host: string): boolean {
  const parts = host.split('.');
  if (parts.length !== 4) return false;
  const nums = parts.map((p) => (/^\d{1,3}$/.test(p) ? Number(p) : -1));
  if (nums.some((n) => n < 0 || n > 255)) return false;

  const [a, b] = nums;
  if (a === 0) return true; // 0.0.0.0/8
  if (a === 10) return true; // 10.0.0.0/8
  if (a === 127) return true; // 127.0.0.0/8 loopback
  if (a === 169 && b === 254) return true; // 169.254.0.0/16 link-local & metadata
  if (a === 172 && b >= 16 && b <= 31) return true; // 172.16.0.0/12
  if (a === 192 && b === 168) return true; // 192.168.0.0/16
  if (a === 100 && b >= 64 && b <= 127) return true; // 100.64.0.0/10 CGNAT
  if (a >= 224) return true; // multicast / reserved
  return false;
}

function isPrivateOrReservedIpv6(host: string): boolean {
  const unwrapped = host.startsWith('[') && host.endsWith(']') ? host.slice(1, -1) : host;
  const lower = unwrapped.toLowerCase();
  if (!lower.includes(':')) return false;
  if (lower === '::' || lower === '::1') return true;
  if (lower.startsWith('fe80:') || lower.startsWith('fe80::')) return true; // link-local
  if (lower.startsWith('fc') || lower.startsWith('fd')) return true; // unique local
  if (lower.startsWith('::ffff:')) {
    const mapped = lower.slice('::ffff:'.length);
    if (isPrivateOrReservedIpv4(mapped)) return true;
  }
  return false;
}

export function validateRelayTargetUri(raw: string): { ok: true; url: URL } | { ok: false; error: string } {
  const trimmed = raw.trim();
  if (!trimmed || trimmed.length > MAX_URI_LEN) {
    return { ok: false, error: 'response_uri が空または長すぎます。' };
  }
  if (/[\s"'\\<>]/.test(trimmed)) {
    return { ok: false, error: 'response_uri に使用できない文字が含まれています。' };
  }
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return { ok: false, error: 'response_uri の URL 形式が不正です。' };
  }
  if (parsed.protocol !== 'https:') {
    return { ok: false, error: 'Relay 送信先は https:// のみ許可されています。' };
  }
  if (parsed.username || parsed.password) {
    return { ok: false, error: 'userinfo を含む URL は許可されていません。' };
  }
  const hostname = parsed.hostname.toLowerCase();
  if (!hostname || hostname === 'localhost' || hostname.endsWith('.localhost') || hostname.endsWith('.local') || hostname.endsWith('.internal')) {
    return { ok: false, error: 'ローカルまたは内部ホストへの Relay 送信は禁止されています。' };
  }
  if (isPrivateOrReservedIpv4(hostname) || isPrivateOrReservedIpv6(hostname)) {
    return { ok: false, error: 'プライベート IP / メタデータ IP への Relay 送信は禁止されています。' };
  }
  return { ok: true, url: parsed };
}

export async function dispatchVcDirectPost(request: Request): Promise<Response | null> {
  const url = new URL(request.url);
  if (url.pathname !== DIRECT_POST_PATH) {
    return null;
  }

  if (request.method !== 'POST') {
    return jsonResponse({ error: 'Method Not Allowed' }, 405);
  }

  let payload: { response_uri?: unknown; form_body?: unknown };
  try {
    payload = (await request.json()) as { response_uri?: unknown; form_body?: unknown };
  } catch {
    return jsonResponse({ error: 'JSON リクエストの解析に失敗しました。' }, 400);
  }

  if (typeof payload.response_uri !== 'string' || typeof payload.form_body !== 'string') {
    return jsonResponse({ error: 'response_uri と form_body は文字列である必要があります。' }, 400);
  }

  if (payload.form_body.length > MAX_FORM_BODY_BYTES) {
    return jsonResponse({ error: 'form_body がサイズ上限 (512 KiB) を超えています。' }, 413);
  }

  const target = validateRelayTargetUri(payload.response_uri);
  if (!target.ok) {
    return jsonResponse({ error: target.error }, 400);
  }

  try {
    const upstream = await fetch(target.url.toString(), {
      method: 'POST',
      headers: {
        'content-type': 'application/x-www-form-urlencoded',
        'user-agent': 'soraneDocs-OID4VP-Relay/1.0',
      },
      body: payload.form_body,
      redirect: 'manual',
    });

    const bodyText = await upstream.text();
    let redirectUri: string | null = upstream.headers.get('location');
    if (!redirectUri && bodyText.trim().startsWith('{')) {
      try {
        const parsed = JSON.parse(bodyText) as { redirect_uri?: unknown };
        if (typeof parsed.redirect_uri === 'string') {
          redirectUri = parsed.redirect_uri;
        }
      } catch {
        // Ignore non-JSON or malformed JSON body
      }
    }

    return jsonResponse({
      status: upstream.status,
      response_uri: target.url.toString(),
      body: bodyText.slice(0, 64 * 1024),
      redirect_uri: redirectUri,
      relayed: true,
    });
  } catch (err) {
    const message = err instanceof Error ? err.message : 'Upstream fetch failed';
    return jsonResponse({ error: `Verifier への Relay 送信に失敗しました: ${message}` }, 502);
  }
}
