// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { createHash, webcrypto } from 'node:crypto';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { generateKeyPair, exportJWK } from 'jose';

globalThis.crypto ??= webcrypto;
const dir = await mkdtemp(join(tmpdir(), 'sorane-files-test-'));
await build({entryPoints:['src/index.ts'],bundle:true,platform:'node',format:'esm',outfile:join(dir,'worker.mjs')});
const {default: worker} = await import(pathToFileURL(join(dir,'worker.mjs')));
const {privateKey} = await generateKeyPair('ES256',{extractable:true});
const jwk = await exportJWK(privateKey); jwk.kid='test-only';
const host='https://docs.example', issuer='https://auth.example';
const digest = s => createHash('sha256').update(s).digest('base64url');
const browser='browser-test', csrf=digest(browser);
const now=()=>Math.floor(Date.now()/1000);
const sample=await readFile(new URL('../../../../www/samples/sorane-welcome.docx',import.meta.url));
const session = (token, sub) => ({token_hash:digest(token),sid:`${sub}-sid`,sub,auth_time:123,lease_until:now()+100,parent_expires_at:now()+3600,idle_expires_at:now()+600,idle_timeout_seconds:600});

function fixture() {
  const objects=new Map(), files=new Map(), versions=new Map();
  let etagSeq=0;
  const env={ISSUER:issuer,RP_ORIGIN:host,CLIENT_ID:'test-client',RP_PRIVATE_JWK:JSON.stringify(jwk),AUTH_LIMITER:{limit:async()=>({success:true})}};
  const matchSql=(sql,start)=>sql.replace(/\s+/g,' ').trim().startsWith(start);
  const db={withSession:()=>db,async batch(statements){const results=[];for(const statement of statements)results.push(await statement.run());return results;},prepare(sql){let args=[];return {bind(...v){args=v;return this},async first(){
    if(matchSql(sql,'SELECT * FROM rp_session')) {
      if(args[0]===digest('owner-token')) return session('owner-token','owner-sub');
      if(args[0]===digest('other-token')) return session('other-token','other-sub');
      return null;
    }
    if(matchSql(sql,'UPDATE rp_session')){
      const token=args[7];
      if(token===digest('owner-token')) return session('owner-token','owner-sub');
      if(token===digest('other-token')) return session('other-token','other-sub');
      return null;
    }
    if(matchSql(sql,'SELECT 1 AS yes')) return null;
    if(matchSql(sql,'SELECT id,issuer,sub,object_key,name,size,etag,created_at,updated_at FROM user_files WHERE id=? AND issuer=? AND sub=?')){
      const row=files.get(args[0]);
      return row && row.issuer===args[1] && row.sub===args[2] ? {...row} : null;
    }
    throw Error('Unexpected query '+sql);
  },async all(){
    if(matchSql(sql,'SELECT id,name,size,etag,updated_at,created_at FROM user_files') || matchSql(sql,'SELECT id,name,size,updated_at FROM user_files')){
      return {results:[...files.values()].filter(row=>row.issuer===args[0] && row.sub===args[1]).sort((a,b)=>b.updated_at-a.updated_at)};
    }
    if(matchSql(sql,'SELECT version_key FROM user_file_versions WHERE id=? ORDER BY created_at DESC')){
      return {results:[...versions.values()].filter(row=>row.id===args[0]).sort((a,b)=>b.created_at-a.created_at || b.version_key.localeCompare(a.version_key)).map(row=>({version_key:row.version_key}))};
    }
    if(matchSql(sql,'SELECT version_key FROM user_file_versions WHERE id=?')){
      return {results:[...versions.values()].filter(row=>row.id===args[0]).map(row=>({version_key:row.version_key}))};
    }
    throw Error('Unexpected all '+sql);
  },async run(){
    if(matchSql(sql,'INSERT INTO user_files')){
      files.set(args[0],{id:args[0],issuer:args[1],sub:args[2],object_key:args[3],name:args[4],size:args[5],etag:args[6],created_at:args[7],updated_at:args[8]});
      return {success:true,meta:{changes:1}};
    }
    if(matchSql(sql,'UPDATE user_files SET size=?')){
      const row=files.get(args[3]);
      if(!row || row.issuer!==args[4] || row.sub!==args[5] || row.etag!==args[6]) return {success:true,meta:{changes:0}};
      row.size=args[0]; row.etag=args[1]; row.updated_at=args[2];
      return {success:true,meta:{changes:1}};
    }
    if(matchSql(sql,'INSERT INTO user_file_versions')){
      versions.set(args[1],{id:args[0],version_key:args[1],size:args[2],etag:args[3],created_at:args[4]});
      return {success:true,meta:{changes:1}};
    }
    if(matchSql(sql,'DELETE FROM user_file_versions WHERE version_key=?')){
      const existed=versions.delete(args[0]);
      return {success:true,meta:{changes:existed?1:0}};
    }
    if(matchSql(sql,'DELETE FROM user_file_versions WHERE id=?')){
      let changes=0;
      for(const [key,row] of versions) if(row.id===args[0]){versions.delete(key);changes++;}
      return {success:true,meta:{changes}};
    }
    if(matchSql(sql,'DELETE FROM user_files WHERE id=? AND issuer=? AND sub=?')){
      const row=files.get(args[0]);
      if(!row || row.issuer!==args[1] || row.sub!==args[2]) return {success:true,meta:{changes:0}};
      files.delete(args[0]);
      return {success:true,meta:{changes:1}};
    }
    throw Error('Unexpected run '+sql);
      }
    };
    }
  };
  const bare=value=>value?.startsWith('"')&&value.endsWith('"')?value.slice(1,-1):value;
  env.DB=db;
  env.FILES={
    async put(key,value,opts={}){
      const bytes=value instanceof ArrayBuffer?new Uint8Array(value):value instanceof Uint8Array?value:new Uint8Array(await new Response(value).arrayBuffer());
      const current=objects.get(key);
      if(opts.onlyIf?.etagDoesNotMatch==='*' && current) return null;
      if(opts.onlyIf?.etagMatches && (!current || current.etag!==bare(opts.onlyIf.etagMatches))) return null;
      const etag=`etag-${++etagSeq}`;
      objects.set(key,{bytes:bytes.slice(),etag,size:bytes.length});
      return {etag,httpEtag:`"${etag}"`,size:bytes.length};
    },
    async get(key){
      const current=objects.get(key);
      if(!current) return null;
      return {body:new Blob([current.bytes]).stream(),arrayBuffer:async()=>current.bytes.slice().buffer,etag:current.etag,httpEtag:`"${current.etag}"`,size:current.size};
    },
    async head(key){
      const current=objects.get(key);
      if(!current) return null;
      return {etag:current.etag,httpEtag:`"${current.etag}"`,size:current.size};
    },
    async delete(keys){
      for(const key of Array.isArray(keys)?keys:[keys]) objects.delete(key);
    },
  };
  env.SAMPLES={async put(){return {etag:'sample'}},async get(){return null}};
  return {env,objects,files,versions};
}
const cookieFor=token=>`__Host-help-browser=${browser}; __Host-help-session=${token}`;
const run=(req,env)=>worker.fetch(req,env,{});
function upload(token,body=sample,name='welcome.docx',extra={}){
  return new Request(host+'/api/files',{method:'POST',headers:{origin:host,cookie:cookieFor(token),'x-csrf-token':csrf,'x-file-name':name,...extra},body});
}
async function create(env,token='owner-token',name='welcome.docx'){
  const response=await run(upload(token,sample,name),env);
  assert.equal(response.status,201,await response.clone().text());
  return response.json();
}

test('file APIs require a session, CSRF and same origin',async()=>{
  const {env}=fixture();
  assert.equal((await run(new Request(host+'/api/files'),env)).status,401);
  assert.equal((await run(upload('missing'),env)).status,401);
  assert.equal((await run(upload('owner-token',sample,'welcome.docx',{origin:'https://attacker.example'}),env)).status,403);
  assert.equal((await run(upload('owner-token',sample,'welcome.docx',{'x-csrf-token':'wrong'}),env)).status,403);
});

test('a user can upload, list, read and delete only their own Office files',async()=>{
  const {env,objects}=fixture();
  assert.equal((await run(upload('owner-token',new Uint8Array([80,75,3,4,1])),env)).status,400);
  assert.equal((await run(upload('owner-token',sample,'My Document.docx'),env)).status,400);
  const created=await create(env);
  const list=await run(new Request(host+'/api/files',{headers:{cookie:cookieFor('owner-token')}}),env);
  assert.equal(list.status,200);
  const listed=await list.json();
  assert.equal(listed.files.length,1);
  assert.equal(listed.files[0].id,created.id);
  assert.equal(listed.files[0].name,'welcome.docx');
  assert.equal(listed.csrf,csrf);
  const otherList=await run(new Request(host+'/api/files',{headers:{cookie:cookieFor('other-token')}}),env);
  assert.equal((await otherList.json()).files.length,0);
  const path=`/api/files/${created.id}/welcome.docx`;
  const downloaded=await run(new Request(host+path,{headers:{cookie:cookieFor('owner-token')}}),env);
  assert.equal(downloaded.status,200);
  assert.equal(downloaded.headers.get('ETag'),created.etag);
  assert.deepEqual(Buffer.from(await downloaded.arrayBuffer()),sample);
  const hidden=await run(new Request(host+path,{headers:{cookie:cookieFor('other-token')}}),env);
  assert.equal(hidden.status,404);
  const removed=await run(new Request(host+path,{method:'DELETE',headers:{origin:host,cookie:cookieFor('owner-token'),'x-csrf-token':csrf}}),env);
  assert.equal(removed.status,204);
  assert.equal(objects.size,0);
  assert.equal((await run(new Request(host+path,{headers:{cookie:cookieFor('owner-token')}}),env)).status,404);
});

test('save requires If-Match and keeps previous versions',async()=>{
  const {env,objects,versions}=fixture();
  const created=await create(env);
  const path=`/api/files/${created.id}/welcome.docx`;
  const unsaved=await run(new Request(host+path,{method:'PUT',headers:{origin:host,cookie:cookieFor('owner-token'),'x-csrf-token':csrf},body:sample}),env);
  assert.equal(unsaved.status,400);
  const stale=await run(new Request(host+path,{method:'PUT',headers:{origin:host,cookie:cookieFor('owner-token'),'x-csrf-token':csrf,'If-Match':'"wrong"'},body:sample}),env);
  assert.equal(stale.status,412);
  const saved=await run(new Request(host+path,{method:'PUT',headers:{origin:host,cookie:cookieFor('owner-token'),'x-csrf-token':csrf,'If-Match':created.etag},body:sample}),env);
  assert.equal(saved.status,200,await saved.clone().text());
  const body=await saved.json();
  assert.notEqual(body.etag,created.etag);
  assert.equal(versions.size,1);
  assert.equal([...objects.keys()].filter(key=>key.startsWith('versions/')).length,1);
  const again=await run(new Request(host+path,{method:'PUT',headers:{origin:host,cookie:cookieFor('owner-token'),'x-csrf-token':csrf,'If-Match':created.etag},body:sample}),env);
  assert.equal(again.status,412);
  let etag=body.etag;
  for(let i=0;i<10;i++){
    const response=await run(new Request(host+path,{method:'PUT',headers:{origin:host,cookie:cookieFor('owner-token'),'x-csrf-token':csrf,'If-Match':etag},body:sample}),env);
    assert.equal(response.status,200,await response.clone().text());
    etag=(await response.json()).etag;
  }
  assert.equal(versions.size,10);
  const leftover=await run(new Request(host+path,{method:'DELETE',headers:{origin:host,cookie:cookieFor('owner-token'),'x-csrf-token':csrf}}),env);
  assert.equal(leftover.status,204);
  assert.equal(objects.size,0);
  assert.equal(versions.size,0);
});

test('the files page asks a visitor to log in',async()=>{
  const {env}=fixture();
  const response=await run(new Request(host+'/files'),env);
  assert.equal(response.status,200);
  const html=await response.text();
  assert.match(html,/mikaki でログイン/);
  assert.match(response.headers.getSetCookie?.().join(',')||response.headers.get('set-cookie')||'',/__Host-sorane-next=\/files/);
});

await rm(dir,{recursive:true,force:true});
