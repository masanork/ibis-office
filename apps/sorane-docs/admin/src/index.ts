// SPDX-License-Identifier: AGPL-3.0-only
import { BROWSER, random, hash, cookieHeader, HttpError, fail, cookie, baseHeaders, redirect, readForm, current, formCsrf, login, callback, backchannel, logout, cleanup } from '../vendor/mikaki-oidc';
import { SAMPLE_MAX_FILE, boundedBody, looksLikeOffice } from './office-file';
import { FILE_PATH, NEXT, deleteFile, filesHome, filesPage, isUserFilesApi, jsonError, listFiles, nextDestination, readFile, saveFile, uploadFile, type FileEnv } from './files';

type Env = FileEnv & { SAMPLES: R2Bucket; AUTH_LIMITER: RateLimit };
function page(env: Env, content: string, status = 200, browser?: string): Response {
  const headers = baseHeaders(env);
  headers.set('Content-Type', 'text/html; charset=utf-8');
  headers.set('Content-Security-Policy', `default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'self' ${env.ISSUER}; frame-ancestors 'none'; base-uri 'none'`);
  if (browser) headers.append('Set-Cookie', cookieHeader(BROWSER, browser, 86400));
  headers.append('Set-Cookie', cookieHeader(NEXT, '/admin', 600));
  return new Response(`<!doctype html><html lang="ja"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>サンプル管理 · Sorane Docs</title><style>body{font:16px system-ui;max-width:46rem;margin:3rem auto;padding:0 1rem;color:#17233d}input,button{font:inherit;padding:.6rem}form{margin:1rem 0}label{display:block;margin:1rem 0}small{color:#53627b}button{cursor:pointer}</style><main><h1>サンプル管理</h1>${content}</main></html>`, { status, headers });
}
async function admin(request: Request, env: Env) {
  const session = await current(request, env, true);
  if (!session) fail(401, 'login_required');
  const allowed = await env.DB.prepare('SELECT 1 AS yes FROM sample_admin WHERE issuer=? AND sub=?').bind(env.ISSUER, session.sub).first();
  if (!allowed) fail(403, 'administrator_required');
  return session;
}
async function enroll(request: Request, env: Env): Promise<Response> {
  const form = await readForm(request, env);
  const session = await current(request, env, true);
  if (!session) fail(401, 'login_required');
  const token = form.get('invitation') ?? '';
  if (!/^[a-zA-Z0-9_-]{43}$/.test(token)) fail(400, 'invalid_invitation');
  const digest = await hash(token), timestamp = Math.floor(Date.now() / 1000);
  // D1 batch is one transaction on a single connection. The DELETE is
  // conditional on the previous INSERT, so a valid invite grants exactly one
  // account even when two redemption requests race.
  const results = await env.DB.batch([
    env.DB.prepare('INSERT INTO sample_admin(issuer,sub,created_at,invite_hash) SELECT ?,?,?,? FROM sample_admin_invite WHERE token_hash=? AND expires_at>?').bind(env.ISSUER,session.sub,timestamp,digest,digest,timestamp),
    env.DB.prepare('DELETE FROM sample_admin_invite WHERE token_hash=? AND changes()=1').bind(digest),
  ]);
  if (results[0].meta.changes !== 1 || results[1].meta.changes !== 1) fail(403, 'invalid_invitation');
  return redirect(env, '/admin');
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
  const bytes = await boundedBody(request, SAMPLE_MAX_FILE);
  if (!looksLikeOffice(bytes, name.toLowerCase())) fail(400, 'invalid_file');
  const id = crypto.randomUUID(), key = `samples/${id}`;
  const object = await env.SAMPLES.put(key, bytes, { httpMetadata: { contentType: 'application/octet-stream' }, onlyIf: { etagDoesNotMatch: '*' } });
  if (!object) fail(409, 'object_exists');
  // A failed D1 response may still have committed. Never delete the object on
  // an ambiguous database result; unreferenced objects can be reconciled later.
  await env.DB.prepare('INSERT INTO samples(id,object_key,name,title,size,created_at,created_by) VALUES(?,?,?,?,?,?,?)').bind(id,key,name,title,bytes.length,Math.floor(Date.now()/1000),session.sub).run();
  return Response.json({ id }, {status:201,headers:baseHeaders(env)});
}
async function finishLogin(request: Request, env: Env, url: URL): Promise<Response> {
  const response = await callback(request, env, url, nextDestination(request));
  const headers = new Headers(response.headers);
  headers.append('Set-Cookie', cookieHeader(NEXT, '', 0));
  return new Response(response.body, { status: response.status, headers });
}
export default {
 async fetch(request: Request, env: Env): Promise<Response> {
  let path = '';
  try {
   const url = new URL(request.url);
   path = url.pathname;
   if (url.origin !== env.RP_ORIGIN || new URL(env.ISSUER).origin !== env.ISSUER || !env.CLIENT_ID || (env.LOCAL_ONLY !== 'true' && (url.protocol !== 'https:' || !env.ISSUER.startsWith('https://')))) fail(400, 'invalid_configuration');
   if (env.LOCAL_ONLY === 'true' && request.method === 'GET' && url.pathname === '/') return redirect(env, '/admin');
   const writingFiles = isUserFilesApi(url.pathname) && ['POST', 'PUT', 'DELETE'].includes(request.method);
   if ((request.method === 'POST' && ['/login','/admin/samples','/admin/enroll'].includes(url.pathname)) || writingFiles) {
    const key = request.headers.get('cf-connecting-ip') || cookie(request,BROWSER) || 'unknown';
    if (!(await env.AUTH_LIMITER.limit({key:`${url.pathname}:${await hash(key)}`})).success) fail(429,'rate_limited');
   }
   if (request.method === 'GET' && url.pathname === '/files') return await filesHome(request, env);
   if (request.method === 'GET' && url.pathname === '/api/files') return await listFiles(request, env);
   if (request.method === 'POST' && url.pathname === '/api/files') return await uploadFile(request, env);
   const filePath = FILE_PATH.exec(url.pathname);
   if (filePath && (request.method === 'GET' || request.method === 'HEAD')) return await readFile(request, env, filePath[1], filePath[2]);
   if (filePath && request.method === 'PUT') return await saveFile(request, env, filePath[1], filePath[2]);
   if (filePath && request.method === 'DELETE') return await deleteFile(request, env, filePath[1], filePath[2]);
   if (request.method === 'GET' && url.pathname === '/admin') {
    const browser = cookie(request,BROWSER) || random(); const session = await current(request,env);
    const csrf = await hash(browser);
    if (!session) return page(env, `<p>サンプルの管理には Mikaki アカウントでログインします。</p><form method="post" action="/login">${formCsrf(csrf)}<button>mikaki でログイン</button></form>`, 200, browser);
    const allowed = await env.DB.prepare('SELECT 1 AS yes FROM sample_admin WHERE issuer=? AND sub=?').bind(env.ISSUER,session.sub).first();
    const body = allowed ? `<p>管理者としてログインしています。登録した文書は公開され、誰でも閲覧できます。</p><form id="upload"><label>タイトル<input name="title" required maxlength="100"></label><label>サンプル（docx, xlsx, pptx、2 MiB 以下）<input type="file" name="file" accept=".docx,.xlsx,.pptx" required></label><button>サンプルとして公開</button></form><p id="result" role="status"></p><script>document.querySelector('#upload').addEventListener('submit',async e=>{e.preventDefault();const f=e.currentTarget;const file=f.elements.namedItem('file').files[0];const title=f.elements.namedItem('title').value;const ext=file?.name.split('.').pop()?.toLowerCase();const result=document.querySelector('#result');if(!file||file.size>2097152||!['docx','xlsx','pptx'].includes(ext)){result.textContent='ファイルのサイズを確認してください。';return}try{const r=await fetch('/admin/samples',{method:'POST',headers:{'x-csrf-token':'${csrf}','x-sample-name':'sample.'+ext,'x-sample-title':encodeURIComponent(title)},body:file});result.textContent=r.ok?'登録しました。':'登録できませんでした。';}catch{result.textContent='通信できませんでした。'}})</script>` : `<p>このアカウントは管理者として登録されていません。運営者からの招待コードがあれば入力してください。</p><form method="post" action="/admin/enroll">${formCsrf(csrf)}<label>招待コード <input name="invitation" type="password" autocomplete="off" required></label><button>管理者を登録</button></form>`;
    return page(env, `${body}<form method="post" action="/logout">${formCsrf(csrf)}<button>ログアウト</button></form>`,200,browser);
   }
   if (request.method === 'POST' && url.pathname === '/login') return await login(request,env,'ja');
   if (request.method === 'GET' && url.pathname === '/callback') return await finishLogin(request,env,url);
   if (request.method === 'POST' && url.pathname === '/logout') return await logout(request,env);
   if (request.method === 'POST' && url.pathname === '/backchannel') return await backchannel(request,env);
   if (request.method === 'POST' && url.pathname === '/admin/samples') return await upload(request,env);
   if (request.method === 'POST' && url.pathname === '/admin/enroll') return await enroll(request,env);
   if (request.method === 'GET' && url.pathname === '/api/samples') {
    const rows = await env.DB.prepare('SELECT id,name,title,size FROM samples ORDER BY created_at DESC LIMIT 100').all();
    return Response.json(rows.results,{headers:{'Cache-Control':'no-store','X-Content-Type-Options':'nosniff'}});
   }
   const samplePath = /^\/api\/samples\/([0-9a-f-]{36})\/([a-zA-Z0-9_-]{1,80}\.(?:docx|xlsx|pptx))$/.exec(url.pathname);
   if (request.method === 'GET' && samplePath) {
    const row = await env.DB.prepare('SELECT object_key,name FROM samples WHERE id=?').bind(samplePath[1]).first<{object_key:string;name:string}>();
    if (!row || row.name !== samplePath[2]) fail(404,'not_found'); const object = await env.SAMPLES.get(row.object_key); if (!object) fail(404,'not_found');
    return new Response(object.body,{headers:{'Content-Type':'application/octet-stream','Content-Disposition':`attachment; filename="${row.name}"`,'X-Content-Type-Options':'nosniff','Cache-Control':'no-store'}});
   }
   fail(404,'not_found');
  } catch(error) {
    const status=error instanceof HttpError?error.status:503;
    if (!(error instanceof HttpError)) console.error(JSON.stringify({event:'admin_request_failed',error:error instanceof Error?error.name:'unknown'}));
    const message=error instanceof HttpError?error.message:'unavailable';
    if (isUserFilesApi(path)) return jsonError(env, status, message);
    if (path === '/files') return filesPage(env,`<p>操作を完了できませんでした。</p><p><a href="/files">保存したファイルへ戻る</a></p>`,status);
    return page(env,`<p>操作を完了できませんでした。</p><p><a href="/admin">管理画面へ戻る</a></p>`,status);
  }
 },
 async scheduled(_event: ScheduledController, env: Env): Promise<void> { await cleanup(env); }
} satisfies ExportedHandler<Env>;
