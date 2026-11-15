// SPDX-License-Identifier: AGPL-3.0-only
import {
  BROWSER,
  cookie,
  cookieHeader,
  current,
  escape,
  fail,
  formCsrf,
  hash,
  random,
  baseHeaders,
  type Session,
} from '../vendor/mikaki-oidc';
import {
  USER_MAX_FILE,
  USER_MAX_VERSIONS,
  bareEtag,
  boundedBody,
  looksLikeOffice,
  quoteEtag,
  safeOfficeName,
} from './office-file';

export type FileEnv = {
  DB: D1Database;
  FILES: R2Bucket;
  ISSUER: string;
  RP_ORIGIN: string;
  CLIENT_ID: string;
  RP_PRIVATE_JWK: string;
  LOCAL_ONLY?: string;
};

type FileRow = {
  id: string;
  issuer: string;
  sub: string;
  object_key: string;
  name: string;
  size: number;
  etag: string;
  created_at: number;
  updated_at: number;
};

export const FILE_PATH =
  /^\/api\/files\/([0-9a-f-]{36})\/([A-Za-z0-9._-]{1,80}\.(?:docx|xlsx|pptx))$/i;

export function isUserFilesApi(path: string): boolean {
  return path === '/api/files' || path.startsWith('/api/files/');
}

export function isUserFilesRoute(path: string): boolean {
  return path === '/files' || isUserFilesApi(path);
}

export function jsonError(env: FileEnv, status: number, message: string): Response {
  return Response.json({ error: message }, { status, headers: baseHeaders(env) });
}

function json(env: FileEnv, body: unknown, status = 200, browser?: string): Response {
  const headers = baseHeaders(env);
  headers.set('Content-Type', 'application/json; charset=utf-8');
  if (browser) headers.append('Set-Cookie', cookieHeader(BROWSER, browser, 86400));
  return new Response(JSON.stringify(body), { status, headers });
}

export function filesPage(env: FileEnv, content: string, status = 200, browser?: string): Response {
  const headers = baseHeaders(env);
  headers.set('Content-Type', 'text/html; charset=utf-8');
  headers.set(
    'Content-Security-Policy',
    `default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'self' ${env.ISSUER}; frame-ancestors 'none'; base-uri 'none'`,
  );
  if (browser) headers.append('Set-Cookie', cookieHeader(BROWSER, browser, 86400));
  headers.append('Set-Cookie', cookieHeader(NEXT, '/files', 600));
  return new Response(
    `<!doctype html><html lang="ja"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>保存したファイル · Sorane Docs</title><style>body{font:16px system-ui;max-width:46rem;margin:3rem auto;padding:0 1rem;color:#17233d}a{color:#1a5fd6}input,button{font:inherit;padding:.6rem}form{margin:1rem 0}label{display:block;margin:1rem 0}small,p.note{color:#53627b}button,a.button{cursor:pointer}ul{list-style:none;padding:0}li{display:flex;gap:1rem;align-items:center;justify-content:space-between;border-bottom:1px solid #e3e7ee;padding:.8rem 0}li form{margin:0}.meta{color:#53627b;font-size:.9rem}#result[data-kind=error]{color:#9b1c1c}</style><main><h1>保存したファイル</h1>${content}</main></html>`,
    { status, headers },
  );
}

export const NEXT = '__Host-sorane-next';

export function nextDestination(request: Request): '/files' | '/admin' {
  const value = cookie(request, NEXT);
  return value === '/files' || value === '/admin' ? value : '/admin';
}

async function requireUser(request: Request, env: FileEnv, forceCheck = false): Promise<Session> {
  const session = await current(request, env, forceCheck);
  if (!session) fail(401, 'login_required');
  return session;
}

async function csrfOk(request: Request, env: FileEnv): Promise<void> {
  if (request.headers.get('origin') !== env.RP_ORIGIN) fail(403, 'invalid_origin');
  const browser = cookie(request, BROWSER);
  if (!browser || request.headers.get('x-csrf-token') !== (await hash(browser)))
    fail(403, 'invalid_csrf');
}

async function ownedFile(env: FileEnv, session: Session, id: string, name: string): Promise<FileRow> {
  const row = await env.DB.prepare(
    'SELECT id,issuer,sub,object_key,name,size,etag,created_at,updated_at FROM user_files WHERE id=? AND issuer=? AND sub=?',
  )
    .bind(id, env.ISSUER, session.sub)
    .first<FileRow>();
  if (!row || row.name !== name) fail(404, 'not_found');
  return row;
}

async function ensureBrowser(request: Request): Promise<{ browser: string; fresh: boolean }> {
  const existing = cookie(request, BROWSER);
  if (existing) return { browser: existing, fresh: false };
  return { browser: random(), fresh: true };
}

export async function listFiles(request: Request, env: FileEnv): Promise<Response> {
  const session = await requireUser(request, env);
  const { browser, fresh } = await ensureBrowser(request);
  const rows = await env.DB.prepare(
    'SELECT id,name,size,etag,updated_at,created_at FROM user_files WHERE issuer=? AND sub=? ORDER BY updated_at DESC LIMIT 100',
  )
    .bind(env.ISSUER, session.sub)
    .all<{ id: string; name: string; size: number; etag: string; updated_at: number; created_at: number }>();
  return json(
    env,
    {
      csrf: await hash(browser),
      files: (rows.results ?? []).map((row) => ({
        id: row.id,
        name: row.name,
        size: row.size,
        etag: quoteEtag(row.etag),
        updated_at: row.updated_at,
        created_at: row.created_at,
      })),
    },
    200,
    fresh ? browser : undefined,
  );
}

export async function uploadFile(request: Request, env: FileEnv): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const name = safeOfficeName(request.headers.get('x-file-name') ?? '');
  if (!name) fail(400, 'invalid_metadata');
  const bytes = await boundedBody(request, USER_MAX_FILE);
  if (!looksLikeOffice(bytes, name.toLowerCase())) fail(400, 'invalid_file');
  const id = crypto.randomUUID();
  const key = `files/${id}/current`;
  const object = await env.FILES.put(key, bytes, {
    httpMetadata: { contentType: 'application/octet-stream' },
    onlyIf: { etagDoesNotMatch: '*' },
  });
  if (!object) fail(409, 'object_exists');
  const etag = bareEtag(object.httpEtag || object.etag);
  const timestamp = Math.floor(Date.now() / 1000);
  await env.DB.prepare(
    'INSERT INTO user_files(id,issuer,sub,object_key,name,size,etag,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)',
  )
    .bind(id, env.ISSUER, session.sub, key, name, bytes.length, etag, timestamp, timestamp)
    .run();
  return json(env, { id, name, size: bytes.length, etag: quoteEtag(etag) }, 201);
}

async function currentObject(env: FileEnv, row: FileRow): Promise<R2ObjectBody> {
  const object = await env.FILES.get(row.object_key);
  if (!object) fail(404, 'not_found');
  return object;
}

function fileHeaders(env: FileEnv, row: FileRow, etag: string): Headers {
  const headers = baseHeaders(env);
  headers.set('Content-Type', 'application/octet-stream');
  headers.set('Content-Disposition', `attachment; filename="${row.name}"`);
  headers.set('ETag', quoteEtag(etag));
  headers.set('Accept-Ranges', 'none');
  return headers;
}

export async function readFile(request: Request, env: FileEnv, id: string, name: string): Promise<Response> {
  const session = await requireUser(request, env);
  const row = await ownedFile(env, session, id, name);
  if (request.method === 'HEAD') {
    const object = await env.FILES.head(row.object_key);
    if (!object) fail(404, 'not_found');
    return new Response(null, { status: 200, headers: fileHeaders(env, row, bareEtag(object.httpEtag || object.etag || row.etag)) });
  }
  const object = await currentObject(env, row);
  const etag = bareEtag(object.httpEtag || object.etag || row.etag);
  return new Response(object.body, { headers: fileHeaders(env, row, etag) });
}

async function archiveCurrent(env: FileEnv, row: FileRow): Promise<void> {
  const current = await currentObject(env, row);
  const timestamp = Date.now();
  const versionKey = `versions/${row.id}/${timestamp}`;
  const body = await current.arrayBuffer();
  const stored = await env.FILES.put(versionKey, body, {
    httpMetadata: { contentType: 'application/octet-stream' },
  });
  if (!stored) fail(503, 'version_write_failed');
  await env.DB.prepare(
    'INSERT INTO user_file_versions(id,version_key,size,etag,created_at) VALUES(?,?,?,?,?)',
  )
    .bind(row.id, versionKey, current.size, bareEtag(current.httpEtag || current.etag || row.etag), Math.floor(timestamp / 1000))
    .run();
  const versions = await env.DB.prepare(
    'SELECT version_key FROM user_file_versions WHERE id=? ORDER BY created_at DESC, version_key DESC',
  )
    .bind(row.id)
    .all<{ version_key: string }>();
  const extra = (versions.results ?? []).slice(USER_MAX_VERSIONS);
  for (const item of extra) {
    await env.FILES.delete(item.version_key);
    await env.DB.prepare('DELETE FROM user_file_versions WHERE version_key=?').bind(item.version_key).run();
  }
}

export async function saveFile(request: Request, env: FileEnv, id: string, name: string): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const row = await ownedFile(env, session, id, name);
  const match = request.headers.get('if-match');
  if (!match) fail(400, 'if_match_required');
  if (match.trim() === '*') fail(400, 'if_match_required');
  const expected = bareEtag(match.trim());
  if (!expected || expected !== row.etag) fail(412, 'precondition_failed');
  const bytes = await boundedBody(request, USER_MAX_FILE);
  if (!looksLikeOffice(bytes, row.name.toLowerCase())) fail(400, 'invalid_file');
  await archiveCurrent(env, row);
  const object = await env.FILES.put(row.object_key, bytes, {
    httpMetadata: { contentType: 'application/octet-stream' },
    onlyIf: { etagMatches: row.etag },
  });
  if (!object) fail(412, 'precondition_failed');
  const etag = bareEtag(object.httpEtag || object.etag);
  const timestamp = Math.floor(Date.now() / 1000);
  const updated = await env.DB.prepare(
    'UPDATE user_files SET size=?, etag=?, updated_at=? WHERE id=? AND issuer=? AND sub=? AND etag=?',
  )
    .bind(bytes.length, etag, timestamp, row.id, env.ISSUER, session.sub, row.etag)
    .run();
  if (updated.meta.changes !== 1) fail(409, 'conflict');
  return json(env, { id: row.id, name: row.name, size: bytes.length, etag: quoteEtag(etag) });
}

export async function deleteFile(request: Request, env: FileEnv, id: string, name: string): Promise<Response> {
  await csrfOk(request, env);
  const session = await requireUser(request, env);
  const row = await ownedFile(env, session, id, name);
  const versions = await env.DB.prepare('SELECT version_key FROM user_file_versions WHERE id=?')
    .bind(row.id)
    .all<{ version_key: string }>();
  const keys = [row.object_key, ...(versions.results ?? []).map((item) => item.version_key)];
  await env.FILES.delete(keys);
  await env.DB.batch([
    env.DB.prepare('DELETE FROM user_file_versions WHERE id=?').bind(row.id),
    env.DB.prepare('DELETE FROM user_files WHERE id=? AND issuer=? AND sub=?').bind(row.id, env.ISSUER, session.sub),
  ]);
  return new Response(null, { status: 204, headers: baseHeaders(env) });
}

export async function filesHome(request: Request, env: FileEnv): Promise<Response> {
  const { browser, fresh } = await ensureBrowser(request);
  const session = await current(request, env);
  const csrf = await hash(browser);
  if (!session) {
    return filesPage(
      env,
      `<p>自分の文書を保存して、ブラウザのエディターで開き、書き戻すには Mikaki アカウントでログインします。</p><form method="post" action="/login">${formCsrf(csrf)}<button>mikaki でログイン</button></form><p class="note"><a href="/">ホームへ戻る</a></p>`,
      200,
      fresh ? browser : undefined,
    );
  }
  const rows = await env.DB.prepare(
    'SELECT id,name,size,updated_at FROM user_files WHERE issuer=? AND sub=? ORDER BY updated_at DESC LIMIT 100',
  )
    .bind(env.ISSUER, session.sub)
    .all<{ id: string; name: string; size: number; updated_at: number }>();
  const items = (rows.results ?? [])
    .map((row) => {
      const open = `/editor.html?open=${encodeURIComponent(`/api/files/${row.id}/${row.name}`)}&download=1`;
      const when = new Date(row.updated_at * 1000).toLocaleString('ja-JP');
      return `<li data-id="${escape(row.id)}" data-name="${escape(row.name)}"><div><a href="${escape(open)}">${escape(row.name)}</a><div class="meta">${row.size} バイト · ${escape(when)}</div></div><div><a class="button" href="${escape(open)}">開く</a> <button type="button" data-delete>削除</button></div></li>`;
    })
    .join('');
  const list = items ? `<ul id="files">${items}</ul>` : `<p>まだ保存したファイルはありません。</p>`;
  const body = `<p>docx、xlsx、pptx を 32 MiB まで保存できます。エディター右上のダウンロードボタンで、いまの内容をここに書き戻します。ブラウザの「ファイル → 保存」はまだクラウドには対応していません。</p><p>ほかの人が同じファイルを更新していた場合、書き戻しは拒否されます。そのときは一覧から開き直してください。</p><form id="upload"><label>ファイル（docx, xlsx, pptx、32 MiB 以下）<input type="file" name="file" accept=".docx,.xlsx,.pptx,application/vnd.openxmlformats-officedocument.wordprocessingml.document,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,application/vnd.openxmlformats-officedocument.presentationml.presentation" required></label><button>アップロード</button></form><p id="result" role="status"></p>${list}<form method="post" action="/logout">${formCsrf(csrf)}<button>ログアウト</button></form><p class="note"><a href="/">ホームへ戻る</a></p><script>
const csrf=${JSON.stringify(csrf)};
const result=document.querySelector('#result');
const say=(text,kind)=>{result.textContent=text;result.dataset.kind=kind||'';};
document.querySelector('#upload').addEventListener('submit',async e=>{
  e.preventDefault();
  const file=e.currentTarget.elements.namedItem('file').files[0];
  if(!file){say('ファイルを選んでください。','error');return;}
  const name=file.name.trim();
  if(!/^[A-Za-z0-9._-]{1,80}\\.(docx|xlsx|pptx)$/i.test(name)||file.size>${USER_MAX_FILE}){say('ファイル名は半角英数と ._-、拡張子は docx / xlsx / pptx、大きさは 32 MiB 以下にしてください。','error');return;}
  try{
    const r=await fetch('/api/files',{method:'POST',headers:{'x-csrf-token':csrf,'x-file-name':name},body:file,credentials:'same-origin'});
    if(r.ok){say('アップロードしました。');location.reload();return;}
    say(r.status===413?'ファイルが大きすぎます。':'アップロードできませんでした。','error');
  }catch{say('通信できませんでした。','error');}
});
document.querySelectorAll('[data-delete]').forEach(button=>button.addEventListener('click',async e=>{
  const item=e.currentTarget.closest('li');
  if(!item||!confirm(item.dataset.name+' を削除しますか。過去の版も削除されます。'))return;
  try{
    const r=await fetch('/api/files/'+item.dataset.id+'/'+encodeURIComponent(item.dataset.name),{method:'DELETE',headers:{'x-csrf-token':csrf},credentials:'same-origin'});
    if(r.status===204){item.remove();say('削除しました。');return;}
    say('削除できませんでした。','error');
  }catch{say('通信できませんでした。','error');}
}));
</script>`;
  return filesPage(env, body, 200, fresh ? browser : undefined);
}
