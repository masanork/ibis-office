// SPDX-License-Identifier: AGPL-3.0-only
import { escape } from '../vendor/mikaki-oidc';
import { normalizeRepoPath } from './git-pack';
import { renderSpecStructure } from './spec-structure';

export function isSpecSource(path: string): boolean {
  return /\.(md|markdown|yaml|yml|json|mmd)$/i.test(path);
}

export function specSourceUrl(repoUrl: string, commit: string, path: string): string {
  return repoUrl + '?' + new URLSearchParams({ ref: commit, source: path }).toString();
}

function resolveSourceLink(source: string, target: string): { path: string; fragment: string } | null {
  if (/^(?:[a-z][a-z0-9+.-]*:|\/|#|\?)/i.test(target)) return null;
  const [pathname, fragment = ''] = target.split('#', 2);
  if (pathname.includes('?')) return null;
  const segments = source.split('/').slice(0, -1);
  try {
    for (const raw of pathname.split('/')) {
      const segment = decodeURIComponent(raw);
      if (!segment || /[\/\\\u0000-\u001f\u007f]/u.test(segment)) return null;
      if (segment === '.') continue;
      if (segment === '..') {
        if (segments.pop() === undefined) return null;
      } else {
        segments.push(segment);
      }
    }
  } catch {
    return null;
  }
  const path = normalizeRepoPath(segments.join('/'));
  if (!path || !isSpecSource(path)) return null;
  return { path, fragment: /^L[1-9]\d*$/.test(fragment) ? '#' + fragment : '' };
}

export function renderSpecSource(source: string, text: string, repoUrl: string, commit: string): string {
  const maxLines = 10_000;
  const lines = text.split(/\r\n|\r|\n/, maxLines + 1);
  const targets = new Map<string, string>();
  if (/\.(md|markdown)$/i.test(source)) {
    let fence: string | undefined;
    for (const line of lines.slice(0, maxLines)) {
      const marker = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
      if (fence) {
        if (marker && marker[1][0] === fence[0] && marker[1].length >= fence.length && !marker[2].trim()) fence = undefined;
        continue;
      }
      if (marker) {
        fence = marker[1];
        continue;
      }
      const candidates = line.matchAll(/(?<![\\!])\[[^\[\]\n]+\]\(\s*(?:<([^<>]+)>|([^\s()]+))(?:\s+["'][^"']*["'])?\s*\)/g);
      for (const [, enclosed, bare] of candidates) {
        const link = resolveSourceLink(source, enclosed ?? bare);
        if (!link) continue;
        const href = specSourceUrl(repoUrl, commit, link.path) + link.fragment;
        targets.set(href, link.path + link.fragment);
        if (targets.size >= 100) break;
      }
      if (targets.size >= 100) break;
    }
  }
  const links = [...targets].map(([href, path]) => `<li><a href="${escape(href)}">${escape(path)}</a></li>`).join('');
  const code = lines.slice(0, maxLines).map((line, i) => {
    const anchor = 'L' + (i + 1);
    return `<span id="${anchor}"><a aria-label="${i + 1} 行目" href="#${anchor}" class="source-line-number muted">${i + 1}</a>  ${escape(line)}</span>`;
  }).join('\n');
  const structure = /\.(yaml|yml|json)$/i.test(source)
    ? renderSpecStructure(text, /\.json$/i.test(source), lines.length)
    : '';
  return `${links ? `<nav aria-label="参照ファイル"><h2>参照ファイル</h2><ul>${links}</ul></nav>` : ''}
    ${structure}<pre class="code source-code"><code>${code}</code></pre>
    ${lines.length > maxLines ? '<p>最初の10,000行を表示しています。全体は Raw で確認できます。</p>' : ''}`;
}
