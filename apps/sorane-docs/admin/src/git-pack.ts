// SPDX-License-Identifier: AGPL-3.0-only
// Minimal pure-TypeScript Git Smart HTTP writer for Cloudflare Workers.
// Builds Git blob/tree/commit objects and PACKv2 payloads using Web Crypto SHA-1
// and CompressionStream('deflate'), then pushes via /git-receive-pack.

const ZERO_SHA = '0000000000000000000000000000000000000000';
const encoder = new TextEncoder();
const decoder = new TextDecoder();

export type GitObjectType = 'blob' | 'tree' | 'commit';

export type TreeEntryInput = {
  mode: string;
  name: string;
  hashHex: string;
  type: 'blob' | 'tree' | 'exec' | 'symlink' | 'gitlink';
};

export type PackObject = {
  type: GitObjectType;
  data: Uint8Array;
};

export type FileChange = {
  path: string;
  /** File content bytes, or `null` to delete the file. */
  content: Uint8Array | null;
};

export function hexToBytes(hex: string): Uint8Array {
  if (!/^[0-9a-f]{40}$/i.test(hex)) {
    throw new Error(`Invalid 40-char SHA-1 hex: ${hex}`);
  }
  const out = new Uint8Array(20);
  for (let i = 0; i < 20; i++) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export function bytesToHex(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i < bytes.length; i++) {
    out += bytes[i].toString(16).padStart(2, '0');
  }
  return out;
}

export function concatBytes(parts: Uint8Array[]): Uint8Array {
  let total = 0;
  for (const p of parts) total += p.length;
  const out = new Uint8Array(total);
  let offset = 0;
  for (const p of parts) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}

export async function sha1Bytes(data: Uint8Array): Promise<Uint8Array> {
  const digest = await crypto.subtle.digest('SHA-1', data as BufferSource);
  return new Uint8Array(digest);
}

export async function hashGitObject(
  type: GitObjectType,
  data: Uint8Array,
): Promise<{ hashHex: string; hashBytes: Uint8Array }> {
  const header = encoder.encode(`${type} ${data.length}\0`);
  const full = concatBytes([header, data]);
  const hashBytes = await sha1Bytes(full);
  return { hashHex: bytesToHex(hashBytes), hashBytes };
}

function compareTreeEntries(a: TreeEntryInput, b: TreeEntryInput): number {
  const aKey = a.type === 'tree' ? `${a.name}/` : a.name;
  const bKey = b.type === 'tree' ? `${b.name}/` : b.name;
  if (aKey < bKey) return -1;
  if (aKey > bKey) return 1;
  return 0;
}

function normalizeGitMode(mode: string, type: TreeEntryInput['type']): string {
  if (type === 'tree') return '40000';
  if (mode === '040000') return '40000';
  return mode || '100644';
}

export function encodeTree(entries: TreeEntryInput[]): Uint8Array {
  const sorted = [...entries].sort(compareTreeEntries);
  const parts: Uint8Array[] = [];
  for (const entry of sorted) {
    const mode = normalizeGitMode(entry.mode, entry.type);
    parts.push(encoder.encode(`${mode} ${entry.name}\0`));
    parts.push(hexToBytes(entry.hashHex));
  }
  return concatBytes(parts);
}

export function encodeCommit(args: {
  treeHash: string;
  parents: string[];
  authorName: string;
  authorEmail: string;
  message: string;
  timestampSec: number;
}): Uint8Array {
  const cleanName = args.authorName.replace(/[\r\n<>]/g, ' ').trim() || 'Docs User';
  const cleanEmail = args.authorEmail.replace(/[\r\n<>]/g, '').trim() || 'docs@sorane.dev';
  const lines = [`tree ${args.treeHash}`];
  for (const p of args.parents) {
    lines.push(`parent ${p}`);
  }
  const ident = `${cleanName} <${cleanEmail}> ${args.timestampSec} +0000`;
  lines.push(`author ${ident}`);
  lines.push(`committer ${ident}`);
  lines.push('');
  lines.push(args.message.endsWith('\n') ? args.message : `${args.message}\n`);
  return encoder.encode(lines.join('\n'));
}

async function deflateZlib(data: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([data as BufferSource]).stream().pipeThrough(new CompressionStream('deflate'));
  const buf = await new Response(stream).arrayBuffer();
  return new Uint8Array(buf);
}

function encodePackObjectHeader(typeNum: number, size: number): Uint8Array {
  const bytes: number[] = [];
  let remaining = size;
  let first = ((typeNum & 0x07) << 4) | (remaining & 0x0f);
  remaining >>>= 4;
  if (remaining > 0) first |= 0x80;
  bytes.push(first);
  while (remaining > 0) {
    let next = remaining & 0x7f;
    remaining >>>= 7;
    if (remaining > 0) next |= 0x80;
    bytes.push(next);
  }
  return new Uint8Array(bytes);
}

const PACK_TYPE_NUM: Record<GitObjectType, number> = {
  commit: 1,
  tree: 2,
  blob: 3,
};

export async function buildPackfile(objects: PackObject[]): Promise<Uint8Array> {
  const header = new Uint8Array(12);
  header.set(encoder.encode('PACK'), 0);
  const view = new DataView(header.buffer);
  view.setUint32(4, 2, false); // PACK version 2
  view.setUint32(8, objects.length, false);

  const parts: Uint8Array[] = [header];
  for (const obj of objects) {
    parts.push(encodePackObjectHeader(PACK_TYPE_NUM[obj.type], obj.data.length));
    parts.push(await deflateZlib(obj.data));
  }
  const body = concatBytes(parts);
  const checksum = await sha1Bytes(body);
  return concatBytes([body, checksum]);
}

export function encodePktLine(payload: string): Uint8Array {
  const body = encoder.encode(payload);
  const lenHex = (body.length + 4).toString(16).padStart(4, '0');
  return concatBytes([encoder.encode(lenHex), body]);
}

export async function pushCommitToRemote(args: {
  remoteUrl: string;
  token: string;
  branch: string;
  oldSha: string;
  newSha: string;
  objects: PackObject[];
}): Promise<void> {
  const auth = `Basic ${btoa(`x-access-token:${args.token}`)}`;
  const refLine = `${args.oldSha} ${args.newSha} refs/heads/${args.branch}\0report-status\n`;
  const pack = await buildPackfile(args.objects);
  const payload = concatBytes([encodePktLine(refLine), encoder.encode('0000'), pack]);

  const receiveUrl = `${args.remoteUrl.replace(/\/+$/, '')}/git-receive-pack`;
  const response = await fetch(receiveUrl, {
    method: 'POST',
    headers: {
      Authorization: auth,
      'Content-Type': 'application/x-git-receive-pack-request',
      Accept: 'application/x-git-receive-pack-result',
    },
    body: payload as BodyInit,
  });

  if (!response.ok) {
    const text = await response.text().catch(() => '');
    throw new Error(`git-receive-pack failed (${response.status}): ${text.slice(0, 200)}`);
  }

  const resBytes = new Uint8Array(await response.arrayBuffer());
  const resText = decoder.decode(resBytes);
  if (resText.includes('unpack ok') === false && resText.includes('ng ')) {
    throw new Error(`git-receive-pack rejected: ${resText.slice(0, 200)}`);
  }
  if (resText.includes(`ng refs/heads/${args.branch}`)) {
    throw new Error(`git-receive-pack ref rejected: ${resText.slice(0, 200)}`);
  }
}

export function normalizeRepoPath(raw: string): string | null {
  const cleaned = raw.replace(/^\/+/, '').replace(/\/+$/, '').trim();
  if (!cleaned || cleaned.length > 240) return null;
  const segments = cleaned.split('/');
  for (const seg of segments) {
    if (!seg || seg === '.' || seg === '..' || /[\0\r\n\\]/.test(seg)) return null;
  }
  return segments.join('/');
}

async function applyTreeChanges(args: {
  repo: ArtifactsRepo;
  treeHash: string | null;
  changes: Map<string, Uint8Array | null>;
  createdObjects: Map<string, PackObject>;
}): Promise<string> {
  const existingEntries: TreeEntryInput[] = args.treeHash
    ? ((await args.repo.readTree(args.treeHash)) ?? []).map((e) => ({
        mode: e.mode,
        name: e.name,
        hashHex: e.hash,
        type: e.type,
      }))
    : [];

  const entryMap = new Map<string, TreeEntryInput>();
  for (const e of existingEntries) {
    entryMap.set(e.name, e);
  }

  // Group changes into direct file changes vs subdirectory changes
  const subChanges = new Map<string, Map<string, Uint8Array | null>>();
  for (const [relPath, content] of args.changes) {
    const slash = relPath.indexOf('/');
    if (slash < 0) {
      if (content === null) {
        entryMap.delete(relPath);
      } else {
        const { hashHex } = await hashGitObject('blob', content);
        if (!args.createdObjects.has(hashHex)) {
          args.createdObjects.set(hashHex, { type: 'blob', data: content });
        }
        entryMap.set(relPath, {
          mode: '100644',
          name: relPath,
          hashHex,
          type: 'blob',
        });
      }
    } else {
      const dir = relPath.slice(0, slash);
      const rest = relPath.slice(slash + 1);
      let sub = subChanges.get(dir);
      if (!sub) {
        sub = new Map();
        subChanges.set(dir, sub);
      }
      sub.set(rest, content);
    }
  }

  for (const [dir, subMap] of subChanges) {
    const prev = entryMap.get(dir);
    const prevTreeHash = prev && prev.type === 'tree' ? prev.hashHex : null;
    const newSubTreeHash = await applyTreeChanges({
      repo: args.repo,
      treeHash: prevTreeHash,
      changes: subMap,
      createdObjects: args.createdObjects,
    });
    entryMap.set(dir, {
      mode: '40000',
      name: dir,
      hashHex: newSubTreeHash,
      type: 'tree',
    });
  }

  const treeBytes = encodeTree([...entryMap.values()]);
  const { hashHex } = await hashGitObject('tree', treeBytes);
  if (!args.createdObjects.has(hashHex)) {
    args.createdObjects.set(hashHex, { type: 'tree', data: treeBytes });
  }
  return hashHex;
}

export class ConflictError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ConflictError';
  }
}

export async function commitFilesToArtifactsRepo(args: {
  repo: ArtifactsRepo;
  branch: string;
  message: string;
  authorName: string;
  authorEmail: string;
  expectedParentSha?: string | null;
  changes: FileChange[];
}): Promise<{ commitHash: string; treeHash: string; parents: string[] }> {
  const info = await args.repo.info();
  const commits = await args.repo.log({ ref: args.branch, limit: 1 });
  const head = commits[0] ?? null;
  const oldSha = head ? head.hash : ZERO_SHA;

  if (args.expectedParentSha !== undefined && args.expectedParentSha !== null) {
    if (args.expectedParentSha !== oldSha) {
      throw new ConflictError(`Expected parent ${args.expectedParentSha}, got ${oldSha}`);
    }
  }

  const changeMap = new Map<string, Uint8Array | null>();
  for (const ch of args.changes) {
    const norm = normalizeRepoPath(ch.path);
    if (!norm) throw new Error(`Invalid repository path: ${ch.path}`);
    changeMap.set(norm, ch.content);
  }

  const createdObjects = new Map<string, PackObject>();
  const treeHash = await applyTreeChanges({
    repo: args.repo,
    treeHash: head ? head.treeHash : null,
    changes: changeMap,
    createdObjects,
  });

  const parents = head ? [head.hash] : [];
  const timestampSec = Math.floor(Date.now() / 1000);
  const commitBytes = encodeCommit({
    treeHash,
    parents,
    authorName: args.authorName,
    authorEmail: args.authorEmail,
    message: args.message,
    timestampSec,
  });
  const { hashHex: commitHash } = await hashGitObject('commit', commitBytes);
  createdObjects.set(commitHash, { type: 'commit', data: commitBytes });

  const tokenResult = await args.repo.createToken('write', 120);
  try {
    await pushCommitToRemote({
      remoteUrl: info.remote,
      token: tokenResult.plaintext,
      branch: args.branch,
      oldSha,
      newSha: commitHash,
      objects: [...createdObjects.values()],
    });
  } finally {
    await args.repo.revokeToken(tokenResult.id).catch(() => {});
  }

  return { commitHash, treeHash, parents };
}
