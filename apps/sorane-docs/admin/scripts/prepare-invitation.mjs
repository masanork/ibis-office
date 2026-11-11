// SPDX-License-Identifier: AGPL-3.0-only
// Prepare, but do not apply, a short-lived invitation in private local files.
import { randomBytes, createHash } from 'node:crypto';
import { mkdtemp, writeFile, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const token = randomBytes(32).toString('base64url');
const digest = createHash('sha256').update(token).digest('base64url');
const now = Math.floor(Date.now() / 1000), expires = now + 15 * 60;
const dir = await mkdtemp(join(tmpdir(), 'sorane-admin-invite-'));
await chmod(dir, 0o700);
await writeFile(join(dir,'invitation.txt'),token+'\n',{mode:0o600,flag:'wx'});
await writeFile(join(dir,'insert.sql'),`INSERT INTO sample_admin_invite(token_hash,expires_at,created_at) VALUES ('${digest}',${expires},${now});\n`,{mode:0o600,flag:'wx'});
console.log(JSON.stringify({directory:dir,expires_at:new Date(expires*1000).toISOString(),applied:false}));
