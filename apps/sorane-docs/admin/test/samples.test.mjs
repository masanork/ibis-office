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
const dir = await mkdtemp(join(tmpdir(), 'sorane-admin-test-'));
await build({entryPoints:['src/index.ts'],bundle:true,platform:'node',format:'esm',outfile:join(dir,'worker.mjs')});
const {default: worker} = await import(pathToFileURL(join(dir,'worker.mjs')));
const {privateKey} = await generateKeyPair('ES256',{extractable:true});
const jwk = await exportJWK(privateKey); jwk.kid='test-only';
const host='https://docs.example', issuer='https://auth.example';
const digest = s => createHash('sha256').update(s).digest('base64url');
const browser='browser-test', csrf=digest(browser);
const now=()=>Math.floor(Date.now()/1000);
const sample=await readFile(new URL('../../../../www/samples/sorane-welcome.docx',import.meta.url));
const oldFetch=globalThis.fetch;
globalThis.fetch=async (url,init) => {
  assert.equal(String(url),issuer+'/session/check');
  const body=JSON.parse(init.body);const sub=body.sid==='admin-sid'?'admin-sub':'other-sub';
  return Response.json({active:true,sub,auth_time:123,expires_at:now()+3600,lease_ttl:300,app_idle_timeout:600});
};
function fixture({role=true,failInsert=false}={}) {
 const objects=new Map(), rows=new Map();
 const env={ISSUER:issuer,RP_ORIGIN:host,CLIENT_ID:'test-client',RP_PRIVATE_JWK:JSON.stringify(jwk),AUTH_LIMITER:{limit:async()=>({success:true})}};
 const db={withSession:()=>db,prepare(sql){let args=[];return {bind(...v){args=v;return this},async first(){
  if(sql.startsWith('SELECT * FROM rp_session')) {let sub=args[0]===digest('admin-token')?'admin-sub':args[0]===digest('other-token')?'other-sub':null; return sub?{token_hash:args[0],sid:sub==='admin-sub'?'admin-sid':'other-sid',sub,auth_time:123,lease_until:now()+100,parent_expires_at:now()+3600,idle_expires_at:now()+600,idle_timeout_seconds:600}:null;}
  if(sql.startsWith('UPDATE rp_session')){let token=args[7], sub=token===digest('admin-token')?'admin-sub':'other-sub';return {token_hash:token,sid:sub==='admin-sub'?'admin-sid':'other-sid',sub,auth_time:123,lease_until:now()+100,parent_expires_at:now()+3600,idle_expires_at:now()+600,idle_timeout_seconds:600};}
  if(sql.startsWith('SELECT 1 AS yes'))return role&&args[1]==='admin-sub'?{yes:1}:null;
  if(sql.startsWith('SELECT object_key'))return rows.get(args[0])??null;
  throw Error('Unexpected query '+sql);
 },async run(){if(sql.startsWith('INSERT INTO samples')) {if(failInsert) throw Error('ambiguous failure');rows.set(args[0],{object_key:args[1],name:args[2],title:args[3],size:args[4]});return {success:true}}throw Error('Unexpected run '+sql)},async all(){return {results:[...rows].map(([id,row])=>({id,...row}))}}}}};
 env.DB=db;env.SAMPLES={async put(k,v){objects.set(k,v);return {etag:'test'}},async get(k){let value=objects.get(k);return value?{body:new Blob([value]).stream()}:null}};
 return {env,objects,rows};
}
function upload(token,body=sample,extra={}) {return new Request(host+'/admin/samples',{method:'POST',headers:{origin:host,cookie:`__Host-help-browser=${browser}; __Host-help-session=${token}`,'x-csrf-token':csrf,'x-sample-name':'welcome.docx','x-sample-title':encodeURIComponent('ようこそ'),...extra},body});}
const run=(req,env)=>worker.fetch(req,env,{});
test('upload requires an active administrator and same-origin CSRF',async()=>{const {env}=fixture();assert.equal((await run(upload('missing'),env)).status,401);assert.equal((await run(upload('other-token'),env)).status,403);assert.equal((await run(upload('admin-token',sample,{origin:'https://attacker.example'}),env)).status,403);assert.equal((await run(upload('admin-token',sample,{'x-csrf-token':'wrong'}),env)).status,403)});
test('a real sample round-trips and malformed ZIP is rejected',async()=>{const {env}=fixture();assert.equal((await run(upload('admin-token',new Uint8Array([80,75,3,4,1])),env)).status,400);let response=await run(upload('admin-token'),env);assert.equal(response.status,201);let {id}=await response.json();let download=await run(new Request(host+'/api/samples/'+id+'/welcome.docx'),env);assert.equal(download.status,200);assert.deepEqual(Buffer.from(await download.arrayBuffer()),sample)});
test('ambiguous database failure does not delete a potentially referenced object',async()=>{const {env,objects}=fixture({failInsert:true});assert.equal((await run(upload('admin-token'),env)).status,503);assert.equal(objects.size,1)});
process.on('exit',()=>{globalThis.fetch=oldFetch;});
await rm(dir,{recursive:true,force:true});
