// SPDX-License-Identifier: AGPL-3.0-only

export type DiffLine = {
  type: 'context' | 'add' | 'del';
  oldLine: number | null;
  newLine: number | null;
  text: string;
};

export type DiffHunk = {
  oldStart: number;
  oldCount: number;
  newStart: number;
  newCount: number;
  lines: DiffLine[];
};

export type LineDiffResult = {
  hunks: DiffHunk[];
  additions: number;
  deletions: number;
  truncated: boolean;
};

export type TreeBlobEntry = {
  hash: string;
  mode: string;
};

export type TreeDiffEntry = {
  path: string;
  status: 'added' | 'modified' | 'deleted';
  oldHash: string | null;
  newHash: string | null;
};

const TEXT_EXTENSIONS = new Set([
  'md',
  'markdown',
  'txt',
  'json',
  'xml',
  'yaml',
  'yml',
  'csv',
  'tsv',
  'html',
  'htm',
  'css',
  'js',
  'ts',
  'toml',
]);

export function isTextDiffablePath(path: string): boolean {
  const dot = path.lastIndexOf('.');
  if (dot < 0) return false;
  return TEXT_EXTENSIONS.has(path.slice(dot + 1).toLowerCase());
}

export function looksLikeBinary(bytes: Uint8Array): boolean {
  const limit = Math.min(bytes.length, 8192);
  for (let i = 0; i < limit; i++) {
    if (bytes[i] === 0) return true;
  }
  return false;
}

function splitLines(text: string): string[] {
  if (text === '') return [];
  const normalized = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
  const lines = normalized.split('\n');
  if (lines.length > 0 && lines[lines.length - 1] === '') {
    lines.pop();
  }
  return lines;
}

type RawOp = {
  type: 'context' | 'add' | 'del';
  oldLine: number | null;
  newLine: number | null;
  text: string;
};

const MAX_DP_CELLS = 400_000;

function computeRawOpsLcs(oldLines: string[], newLines: string[]): RawOp[] {
  const n = oldLines.length;
  const m = newLines.length;
  const width = m + 1;
  const dp = new Int32Array((n + 1) * width);

  for (let i = n - 1; i >= 0; i--) {
    const row = i * width;
    const nextRow = (i + 1) * width;
    for (let j = m - 1; j >= 0; j--) {
      if (oldLines[i] === newLines[j]) {
        dp[row + j] = dp[nextRow + j + 1] + 1;
      } else {
        const down = dp[nextRow + j];
        const right = dp[row + j + 1];
        dp[row + j] = down >= right ? down : right;
      }
    }
  }

  const ops: RawOp[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (oldLines[i] === newLines[j]) {
      ops.push({ type: 'context', oldLine: i + 1, newLine: j + 1, text: oldLines[i] });
      i++;
      j++;
    } else if (dp[(i + 1) * width + j] >= dp[i * width + j + 1]) {
      ops.push({ type: 'del', oldLine: i + 1, newLine: null, text: oldLines[i] });
      i++;
    } else {
      ops.push({ type: 'add', oldLine: null, newLine: j + 1, text: newLines[j] });
      j++;
    }
  }
  while (i < n) {
    ops.push({ type: 'del', oldLine: i + 1, newLine: null, text: oldLines[i] });
    i++;
  }
  while (j < m) {
    ops.push({ type: 'add', oldLine: null, newLine: j + 1, text: newLines[j] });
    j++;
  }
  return ops;
}

function buildHunks(ops: RawOp[], contextLines: number): DiffHunk[] {
  const changedIndices: number[] = [];
  for (let idx = 0; idx < ops.length; idx++) {
    if (ops[idx].type !== 'context') changedIndices.push(idx);
  }
  if (changedIndices.length === 0) return [];

  const ranges: Array<[number, number]> = [];
  for (const idx of changedIndices) {
    const start = Math.max(0, idx - contextLines);
    const end = Math.min(ops.length - 1, idx + contextLines);
    const prev = ranges[ranges.length - 1];
    if (prev && start <= prev[1] + 1) {
      prev[1] = Math.max(prev[1], end);
    } else {
      ranges.push([start, end]);
    }
  }

  return ranges.map(([start, end]) => {
    const slice = ops.slice(start, end + 1);
    let oldStart = 0;
    let newStart = 0;
    let oldCount = 0;
    let newCount = 0;
    for (const op of slice) {
      if (op.oldLine !== null) {
        if (oldStart === 0) oldStart = op.oldLine;
        oldCount++;
      }
      if (op.newLine !== null) {
        if (newStart === 0) newStart = op.newLine;
        newCount++;
      }
    }
    return { oldStart, oldCount, newStart, newCount, lines: slice };
  });
}

export function computeLineDiff(
  oldText: string,
  newText: string,
  contextLines = 3,
): LineDiffResult {
  const oldLines = splitLines(oldText);
  const newLines = splitLines(newText);

  if (oldLines.length === 0 && newLines.length === 0) {
    return { hunks: [], additions: 0, deletions: 0, truncated: false };
  }

  // Trim identical prefix and suffix before running LCS DP so large files with
  // localized edits stay well under the DP cell budget.
  let prefix = 0;
  const minLen = Math.min(oldLines.length, newLines.length);
  while (prefix < minLen && oldLines[prefix] === newLines[prefix]) {
    prefix++;
  }

  let suffix = 0;
  while (
    suffix < minLen - prefix &&
    oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
  ) {
    suffix++;
  }

  const midOld = oldLines.slice(prefix, oldLines.length - suffix);
  const midNew = newLines.slice(prefix, newLines.length - suffix);

  if (midOld.length === 0 && midNew.length === 0) {
    return { hunks: [], additions: 0, deletions: 0, truncated: false };
  }

  let truncated = false;
  let midOps: RawOp[];
  if (midOld.length * midNew.length > MAX_DP_CELLS) {
    truncated = true;
    midOps = [
      ...midOld.map((text, idx) => ({
        type: 'del' as const,
        oldLine: prefix + idx + 1,
        newLine: null,
        text,
      })),
      ...midNew.map((text, idx) => ({
        type: 'add' as const,
        oldLine: null,
        newLine: prefix + idx + 1,
        text,
      })),
    ];
  } else {
    midOps = computeRawOpsLcs(midOld, midNew).map((op) => ({
      ...op,
      oldLine: op.oldLine !== null ? op.oldLine + prefix : null,
      newLine: op.newLine !== null ? op.newLine + prefix : null,
    }));
  }

  const allOps: RawOp[] = [];
  const ctxStart = Math.max(0, prefix - contextLines);
  for (let idx = ctxStart; idx < prefix; idx++) {
    allOps.push({ type: 'context', oldLine: idx + 1, newLine: idx + 1, text: oldLines[idx] });
  }
  allOps.push(...midOps);
  const ctxEnd = Math.min(suffix, contextLines);
  for (let k = 0; k < ctxEnd; k++) {
    const oldIdx = oldLines.length - suffix + k;
    const newIdx = newLines.length - suffix + k;
    allOps.push({
      type: 'context',
      oldLine: oldIdx + 1,
      newLine: newIdx + 1,
      text: oldLines[oldIdx],
    });
  }

  let additions = 0;
  let deletions = 0;
  for (const op of allOps) {
    if (op.type === 'add') additions++;
    else if (op.type === 'del') deletions++;
  }

  return {
    hunks: buildHunks(allOps, contextLines),
    additions,
    deletions,
    truncated,
  };
}

export async function collectTreeBlobs(
  repo: Pick<ArtifactsRepo, 'readTree'>,
  treeHash: string | null,
  prefix = '',
  out = new Map<string, TreeBlobEntry>(),
): Promise<Map<string, TreeBlobEntry>> {
  if (!treeHash) return out;
  const entries = (await repo.readTree(treeHash)) ?? [];
  for (const entry of entries) {
    const fullPath = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.type === 'tree') {
      await collectTreeBlobs(repo, entry.hash, fullPath, out);
    } else if (entry.type === 'blob') {
      out.set(fullPath, { hash: entry.hash, mode: entry.mode });
    }
  }
  return out;
}

export async function diffCommitTrees(
  repo: Pick<ArtifactsRepo, 'readTree'>,
  baseTreeHash: string | null,
  headTreeHash: string | null,
  prefix = '',
  out: TreeDiffEntry[] = [],
): Promise<TreeDiffEntry[]> {
  if (baseTreeHash === headTreeHash) return out;

  const baseEntries = baseTreeHash ? ((await repo.readTree(baseTreeHash)) ?? []) : [];
  const headEntries = headTreeHash ? ((await repo.readTree(headTreeHash)) ?? []) : [];

  const baseMap = new Map(baseEntries.map((e) => [e.name, e]));
  const headMap = new Map(headEntries.map((e) => [e.name, e]));
  const names = [...new Set([...baseMap.keys(), ...headMap.keys()])].sort((a, b) =>
    a.localeCompare(b),
  );

  for (const name of names) {
    const b = baseMap.get(name);
    const h = headMap.get(name);
    const fullPath = prefix ? `${prefix}/${name}` : name;

    if (b && h && b.hash === h.hash && b.type === h.type) continue;

    if (b?.type === 'tree' && h?.type === 'tree') {
      await diffCommitTrees(repo, b.hash, h.hash, fullPath, out);
      continue;
    }

    if (b?.type === 'tree') {
      const removed = await collectTreeBlobs(repo, b.hash, fullPath);
      for (const [p, entry] of removed) {
        out.push({ path: p, status: 'deleted', oldHash: entry.hash, newHash: null });
      }
      if (h?.type === 'blob') {
        out.push({ path: fullPath, status: 'added', oldHash: null, newHash: h.hash });
      }
      continue;
    }

    if (h?.type === 'tree') {
      if (b?.type === 'blob') {
        out.push({ path: fullPath, status: 'deleted', oldHash: b.hash, newHash: null });
      }
      const added = await collectTreeBlobs(repo, h.hash, fullPath);
      for (const [p, entry] of added) {
        out.push({ path: p, status: 'added', oldHash: null, newHash: entry.hash });
      }
      continue;
    }

    if (!b && h?.type === 'blob') {
      out.push({ path: fullPath, status: 'added', oldHash: null, newHash: h.hash });
    } else if (b?.type === 'blob' && !h) {
      out.push({ path: fullPath, status: 'deleted', oldHash: b.hash, newHash: null });
    } else if (b?.type === 'blob' && h?.type === 'blob') {
      out.push({ path: fullPath, status: 'modified', oldHash: b.hash, newHash: h.hash });
    }
  }

  out.sort((a, b) => a.path.localeCompare(b.path));
  return out;
}
