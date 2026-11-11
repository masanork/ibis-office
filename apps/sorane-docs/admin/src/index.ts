// SPDX-License-Identifier: AGPL-3.0-only
import { BROWSER, random, hash, escape, cookieHeader, HttpError, fail, cookie, baseHeaders, redirect, readForm, current, formCsrf, login, callback, backchannel, logout, cleanup } from '../vendor/mikaki-oidc';

type Env = { DB: D1Database; SAMPLES: R2Bucket; AUTH_LIMITER: RateLimit; ISSUER: string; RP_ORIGIN: string; CLIENT_ID: string; RP_PRIVATE_JWK: string; LOCAL_ONLY?: string };
const maxFile = 2 * 1024 * 1024;
function page(env: Env, content: string, status = 200, browser?: string): Response {
  const headers = baseHeaders(env);
  headers.set('Content-Type', 'text/html; charset=utf-8');
  headers.set('Content-Security-Policy', `default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'self' ${env.ISSUER}; frame-ancestors 'none'; base-uri 'none'`);
  if (browser) headers.append('Set-Cookie', cookieHeader(BROWSER, browser, 86400));
  return new Response(`<!doctype html><html lang="ja"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>サンプル管理 · Sorane Docs</title><style>body{font:16px system-ui;max-width:46rem;margin:3rem auto;padding:0 1rem;color:#17233d}input,button{font:inherit;padding:.6rem}form{margin:1rem 0}label{display:block;margin:1rem 0}small{color:#53627b}button{cursor:pointer}</style><main><h1>サンプル管理</h1>${content}</main></html>`, { status, headers });
}
async function admin(request: Request, env: Env) {
  const session = await current(request, env, true);
  if (!session) fail(401, 'login_required');
  const allowed = await env.DB.prepare('SELECT 1 AS yes FROM sample_admin WHERE issuer=? AND sub=?').bind(env.ISSUER, session.sub).first();
  if (!allowed) fail(403, 'administrator_required');
  return session;
}
async function boundedBody(request: Request): Promise<Uint8Array> {
  const declared = Number(request.headers.get('content-length'));
  if (Number.isFinite(declared) && declared > maxFile) fail(413, 'file_too_large');
  if (!request.body) fail(400, 'missing_file');
  const reader = request.body.getReader(); const chunks: Uint8Array[] = []; let length = 0;
  while (true) { const {done,value} = await reader.read(); if (done) break; length += value.length; if (length > maxFile) { await reader.cancel(); fail(413, 'file_too_large'); } chunks.push(value); }
  const body = new Uint8Array(length); let offset = 0; for (const chunk of chunks) { body.set(chunk, offset); offset += chunk.length; } return body;
}
// Check the ZIP directory without expanding uploaded data. This is a bounded
// structural filter, not a full OOXML validator or malware scan.
function looksLikeOffice(bytes: Uint8Array, name: string): boolean {
  if (bytes.length < 22) return false;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const u16 = (p: number) => view.getUint16(p, true), u32 = (p: number) => view.getUint32(p, true);
  let end = -1;
  for (let p = bytes.length - 22; p >= Math.max(0, bytes.length - 65557); p--) {
    if (u32(p) === 0x06054b50 && p + 22 + u16(p + 20) === bytes.length) { end = p; break; }
  }
  if (end < 0 || u16(end + 4) !== 0 || u16(end + 6) !== 0) return false;
  const count = u16(end + 10), size = u32(end + 12), start = u32(end + 16);
  if (count === 0xffff || size === 0xffffffff || start === 0xffffffff || start + size !== end) return false;
  const expected = name.endsWith('.docx') ? 'word/document.xml' : name.endsWith('.xlsx') ? 'xl/workbook.xml' : 'ppt/presentation.xml';
  let p = start, contentTypes = false, document = false;
  for (let i = 0; i < count; i++) {
    if (p + 46 > end || u32(p) !== 0x02014b50 || (u16(p + 8) & 1) !== 0) return false;
    const fileLength = u16(p + 28), extra = u16(p + 30), comment = u16(p + 32), next = p + 46 + fileLength + extra + comment;
    if (next > end) return false;
    const path = new TextDecoder().decode(bytes.subarray(p + 46, p + 46 + fileLength));
    contentTypes ||= path === '[Content_Types].xml'; document ||= path === expected; p = next;
  }
  return p === end && contentTypes && document;
}
async function upload(request: Request, env: Env) {
  if (request.headers.get('origin') !== env.RP_ORIGIN) fail(403, 'invalid_origin');
  const browser = cookie(request, BROWSER);
  if (!browser || request.headers.get('x-csrf-token') !== await hash(browser)) fail(403, 'invalid_csrf');
  const session = await admin(request, env);
  const name = request.headers.get('x-sample-name') ?? '';
  let title = '';
  try { title = decodeURIComponent(request.headers.get('x-sample-title') ?? '').trim(); } catch { fail(400, 'invalid_metadata'); }
  if (!/^[a-zA-Z0-9_-]{1,80}\.(docx|xlsx|pptx)$/i.test(name) || !title || title.length > 100 || /[\u0000-\u001f\u007f]/.test(title)) fail(400, 'invalid_metadata');
  const bytes = await boundedBody(request);
  if (!looksLikeOffice(bytes, name.toLowerCase())) fail(400, 'invalid_file');
  const id = crypto.randomUUID(), key = `samples/${id}`;
  const object = await env.SAMPLES.put(key, bytes, { httpMetadata: { contentType: 'application/octet-stream' }, onlyIf: { etagDoesNotMatch: '*' } });
  if (!object) fail(409, 'object_exists');
  // A failed D1 response may still have committed. Never delete the object on
  // an ambiguous database result; unreferenced objects can be reconciled later.
  await env.DB.prepare('INSERT INTO samples(id,object_key,name,title,size,created_at,created_by) VALUES(?,?,?,?,?,?,?)').bind(id,key,name,title,bytes.length,Math.floor(Date.now()/1000),session.sub).run();
  return Response.json({ id }, {status:201,headers:baseHeaders(env)});
}
export default {
 async fetch(request: Request, env: Env): Promise<Response> {
  try {
   const url = new URL(request.url);
   if (url.origin !== env.RP_ORIGIN || new URL(env.ISSUER).origin !== env.ISSUER || !env.CLIENT_ID || (env.LOCAL_ONLY !== 'true' && (url.protocol !== 'https:' || !env.ISSUER.startsWith('https://')))) fail(400, 'invalid_configuration');
   if (env.LOCAL_ONLY === 'true' && request.method === 'GET' && url.pathname === '/') return redirect(env, '/admin');
   if (request.method === 'POST' && ['/login','/admin/samples'].includes(url.pathname)) {
    const key = request.headers.get('cf-connecting-ip') || cookie(request,BROWSER) || 'unknown';
    if (!(await env.AUTH_LIMITER.limit({key:`${url.pathname}:${await hash(key)}`})).success) fail(429,'rate_limited');
   }
   if (request.method === 'GET' && url.pathname === '/admin') {
    const browser = cookie(request,BROWSER) || random(); const session = await current(request,env);
    const csrf = await hash(browser);
    if (!session) return page(env, `<p>サンプルの管理には Mikaki アカウントでログインします。</p><form method="post" action="/login">${formCsrf(csrf)}<button>mikaki でログイン</button></form>`, 200, browser);
    const allowed = await env.DB.prepare('SELECT 1 AS yes FROM sample_admin WHERE issuer=? AND sub=?').bind(env.ISSUER,session.sub).first();
    const body = allowed ? `<p>管理者としてログインしています。</p><form id="upload"><label>タイトル<input name="title" required maxlength="100"></label><label>サンプル（docx, xlsx, pptx、2 MiB 以下）<input type="file" name="file" accept=".docx,.xlsx,.pptx" required></label><button>登録</button></form><p id="result" role="status"></p><script>document.querySelector('#upload').addEventListener('submit',async e=>{e.preventDefault();const f=e.currentTarget;const file=f.elements.namedItem('file').files[0];const title=f.elements.namedItem('title').value;const ext=file?.name.split('.').pop()?.toLowerCase();const result=document.querySelector('#result');if(!file||file.size>2097152||!['docx','xlsx','pptx'].includes(ext)){result.textContent='ファイルのサイズを確認してください。';return}try{const r=await fetch('/admin/samples',{method:'POST',headers:{'x-csrf-token':'${csrf}','x-sample-name':'sample.'+ext,'x-sample-title':encodeURIComponent(title)},body:file});result.textContent=r.ok?'登録しました。':'登録できませんでした。';}catch{result.textContent='通信できませんでした。'}})</script>` : `<p>このアカウントは管理者として登録されていません。</p><p>管理者の登録を依頼する場合は、次の識別子を運営者に伝えてください。</p><code>${escape(session.sub)}</code>`;
    return page(env, `${body}<form method="post" action="/logout">${formCsrf(csrf)}<button>ログアウト</button></form>`,200,browser);
   }
   if (request.method === 'POST' && url.pathname === '/login') return await login(request,env,'ja');
   if (request.method === 'GET' && url.pathname === '/callback') return await callback(request,env,url,'/admin');
   if (request.method === 'POST' && url.pathname === '/logout') return await logout(request,env);
   if (request.method === 'POST' && url.pathname === '/backchannel') return await backchannel(request,env);
   if (request.method === 'POST' && url.pathname === '/admin/samples') return await upload(request,env);
   if (request.method === 'GET' && url.pathname === '/samples') {
    const rows = await env.DB.prepare('SELECT id,name,title,size FROM samples ORDER BY created_at DESC LIMIT 100').all();
    return Response.json(rows.results,{headers:{'Cache-Control':'no-store','X-Content-Type-Options':'nosniff'}});
   }
   const samplePath = /^\/samples\/([0-9a-f-]{36})\/([a-zA-Z0-9_-]{1,80}\.(?:docx|xlsx|pptx))$/.exec(url.pathname);
   if (request.method === 'GET' && samplePath) {
    const row = await env.DB.prepare('SELECT object_key,name FROM samples WHERE id=?').bind(samplePath[1]).first<{object_key:string;name:string}>();
    if (!row || row.name !== samplePath[2]) fail(404,'not_found'); const object = await env.SAMPLES.get(row.object_key); if (!object) fail(404,'not_found');
    return new Response(object.body,{headers:{'Content-Type':'application/octet-stream','Content-Disposition':`attachment; filename="${row.name}"`,'X-Content-Type-Options':'nosniff','Cache-Control':'no-store'}});
   }
   fail(404,'not_found');
  } catch(error) { const status=error instanceof HttpError?error.status:503; if (!(error instanceof HttpError)) console.error(JSON.stringify({event:'admin_request_failed',error:error instanceof Error?error.name:'unknown'})); return page(env,`<p>操作を完了できませんでした。</p><p><a href="/admin">管理画面へ戻る</a></p>`,status); }
 },
 async scheduled(_event: ScheduledController, env: Env): Promise<void> { await cleanup(env); }
} satisfies ExportedHandler<Env>;
