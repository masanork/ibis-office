// SPDX-License-Identifier: AGPL-3.0-only
import { fail } from '../vendor/mikaki-oidc';

type Identity = { name: string | null; email: string | null; date: string };
type Commit = {
  sha: string;
  commit: { message: string; tree: { sha: string }; author: Identity | null; committer: Identity | null };
  parents: { sha: string }[];
};

const OWNER = '[A-Za-z0-9][A-Za-z0-9-]{0,38}';
const NAME = '[A-Za-z0-9_.-]{1,100}';
const coordinates = new RegExp('^/(' + OWNER + ')/(' + NAME + ')$');
const shaPattern = /^[0-9a-f]{40}$/;

export function githubCoordinates(path: string): { owner: string; name: string } | null {
  const match = coordinates.exec(path);
  if (!match || match[2] === '.' || match[2] === '..') return null;
  return { owner: match[1], name: match[2] };
}

export function githubUrlCoordinates(input: string) {
  try {
    const url = new URL(input);
    if (url.origin !== 'https://github.com' || url.username || url.password || url.search || url.hash) return null;
    const path = url.pathname.endsWith('/') ? url.pathname.slice(0, -1) : url.pathname;
    return githubCoordinates(path.endsWith('.git') ? path.slice(0, -4) : path);
  } catch {
    return null;
  }
}

function metadata(row: Commit): ArtifactsCommitMetadata {
  const identity = (person: Identity | null) => ({ name: person?.name ?? '', email: person?.email ?? '' });
  const timestamp = (person: Identity | null) => person ? Date.parse(person.date) / 1000 : 0;
  return {
    hash: row.sha,
    treeHash: row.commit.tree.sha,
    message: row.commit.message,
    author: identity(row.commit.author),
    committer: identity(row.commit.committer),
    authoredAt: timestamp(row.commit.author),
    committedAt: timestamp(row.commit.committer),
    parents: row.parents.map((p) => p.sha),
  };
}

export class GitHubRepo {
  readonly name: string;
  readonly remote: string;
  private readonly api: string;
  private budget = 38;

  constructor(owner: string, name: string, private readonly maxDiffBytes: number) {
    this.name = owner + '/' + name;
    this.remote = 'https://github.com/' + this.name;
    this.api = 'https://api.github.com/repos/' + this.name;
  }

  private async request(url: string, accept: string, immutable = false) {
    if (--this.budget < 0) fail(422, 'narrow_to_file');
    const response = await fetch(url, {
      redirect: 'error',
      headers: { Accept: accept, 'User-Agent': 'Sorane-Docs' },
      cf: { cacheEverything: true, cacheTtlByStatus: { '200-299': immutable ? 86400 : 120, '400-599': 0 } },
    }).catch(() => fail(502, 'github_unavailable'));
    if (response.status === 404 || response.status === 409) return null;
    if (response.status === 429 || response.status === 403) fail(429, 'github_rate_limit');
    if (!response.ok) fail(502, 'github_unavailable');
    return response;
  }

  private get(path: string, immutable = false) {
    return this.request(this.api + path, 'application/vnd.github+json', immutable);
  }

  async info() {
    const result = await this.get('');
    if (!result) fail(404, 'not_found');
    const data = await result.json<{ private: boolean; default_branch: string; description: string | null }>();
    if (data.private) fail(404, 'not_found');
    return { defaultBranch: data.default_branch, description: data.description };
  }

  async branches() {
    const result = await this.get('/branches?per_page=100');
    const branches = result ? await result.json<{ name: string; commit: { sha: string } }[]>() : [];
    return branches.map((branch) => ({ name: branch.name, sha: branch.commit.sha }));
  }

  async readCommit(ref: string) {
    if (!ref || ref === '.' || ref === '..' || ref.length > 240) return null;
    const result = await this.get('/commits/' + encodeURIComponent(ref), shaPattern.test(ref));
    return result ? metadata(await result.json<Commit>()) : null;
  }

  async log({ ref }: { ref: string; limit: number }) {
    const commit = await this.readCommit(ref);
    return commit ? [commit] : [];
  }

  async history(ref: string, path: string | null, page: number) {
    const params = new URLSearchParams({ sha: ref, page: String(page), per_page: '30' });
    if (path) params.set('path', path);
    const result = await this.get('/commits?' + params, shaPattern.test(ref));
    const commits = result ? await result.json<Commit[]>() : [];
    return {
      rows: commits.map((commit) => ({ commit: metadata(commit), status: null })),
      next: result?.headers.get('link')?.includes('rel="next"') ? String(page + 1) : undefined,
    };
  }

  async readTree(hash: string): Promise<ArtifactsTreeEntry[] | null> {
    if (!shaPattern.test(hash)) return null;
    const result = await this.get('/git/trees/' + hash, true);
    if (!result) return null;
    const tree = await result.json<{
      truncated?: boolean;
      tree: { path: string; mode: string; sha: string; type: 'blob' | 'tree' | 'commit' }[];
    }>();
    if (tree.truncated) fail(422, 'github_tree_truncated');
    return tree.tree.map((entry) => ({
      name: entry.path,
      hash: entry.sha,
      mode: entry.mode === '040000' ? '40000' : entry.mode,
      type: entry.type === 'commit' ? 'gitlink'
        : entry.mode === '120000' ? 'symlink'
        : entry.mode === '100755' ? 'exec' : entry.type,
    }));
  }

  async readBlob(hash: string) {
    if (!shaPattern.test(hash)) return null;
    const result = await this.request(this.api + '/git/blobs/' + hash, 'application/vnd.github.raw+json', true);
    if (!result) fail(404, 'not_found');
    const ceiling = this.maxDiffBytes + 1;
    const length = Number(result.headers.get('Content-Length'));
    if (length >= ceiling) {
      await result.body?.cancel();
      return { size: length, arrayBuffer: async () => new ArrayBuffer(0) };
    }
    const reader = result.body?.getReader();
    if (!reader) return new Blob();
    const bytes = new Uint8Array(ceiling);
    let offset = 0;
    try {
      while (offset < ceiling) {
        const chunk = await reader.read();
        if (chunk.done) break;
        const part = chunk.value.subarray(0, ceiling - offset);
        bytes.set(part, offset);
        offset += part.length;
      }
    } finally {
      await reader.cancel();
    }
    return new Blob([bytes.subarray(0, offset)]);
  }

  async raw(sha: string, path: string) {
    if (!shaPattern.test(sha)) fail(404, 'not_found');
    return this.request(
      'https://raw.githubusercontent.com/' + this.name + '/' + sha + '/' +
        path.split('/').map(encodeURIComponent).join('/'),
      '*/*', true,
    );
  }
}
