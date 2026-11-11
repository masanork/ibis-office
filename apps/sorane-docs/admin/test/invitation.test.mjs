// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFile, stat, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';

test('invitation preparation writes private files without printing the token',async()=>{
 const output=execFileSync(process.execPath,['scripts/prepare-invitation.mjs'],{encoding:'utf8'});
 const result=JSON.parse(output);try{
  assert.equal(result.applied,false);const token=(await readFile(join(result.directory,'invitation.txt'),'utf8')).trim();
  assert.match(token,/^[A-Za-z0-9_-]{43}$/);assert.equal(output.includes(token),false);
  assert.equal((await stat(result.directory)).mode&0o777,0o700);assert.equal((await stat(join(result.directory,'invitation.txt'))).mode&0o777,0o600);
  const sql=await readFile(join(result.directory,'insert.sql'),'utf8');assert.ok(sql.includes(createHash('sha256').update(token).digest('base64url')));assert.equal(sql.includes(token),false);
 }finally{await rm(result.directory,{recursive:true,force:true})}
});
