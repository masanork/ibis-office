// SPDX-License-Identifier: AGPL-3.0-only
import { escape } from '../vendor/mikaki-oidc';
import { resolveRelativePath, schemaReference } from './spec-reference';
import { renderSpecStructure } from './spec-structure';
import { renderGherkinOutline } from './spec-gherkin';

export function isSpecSource(path: string): boolean {
  return /\.(md|markdown|yaml|yml|json|mmd|feature)$/i.test(path);
}

export function specSourceUrl(repoUrl: string, commit: string, path: string): string {
  return repoUrl + '?' + new URLSearchParams({ ref: commit, source: path }).toString();
}

function referenceUrl(repoUrl: string, commit: string, source: string, value: string): string | null {
  const target = schemaReference(source, value);
  if (!target) return null;
  const url = specSourceUrl(repoUrl, commit, target.path);
  return target.pointer ? url + '&' + new URLSearchParams({ pointer: target.pointer }) + '#definition' : url;
}

export async function renderSpecSource(source: string, text: string, repoUrl: string, commit: string, pointer: string | null, loadSchema: (path: string) => Promise<string | null>): Promise<string> {
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
        const [pathname, hash = ''] = (enclosed ?? bare).split('#', 2);
        const path = resolveRelativePath(source, pathname);
        if (!path || !isSpecSource(path)) continue;
        const fragment = /^L[1-9]\d*$/.test(hash) ? '#' + hash : '';
        const href = specSourceUrl(repoUrl, commit, path) + fragment;
        targets.set(href, path + fragment);
        if (targets.size >= 100) break;
      }
      if (targets.size >= 100) break;
    }
  }
  const links = [...targets].map(([href, path]) => `<li><a href="${escape(href)}">${escape(path)}</a></li>`).join('');
  const structure = /\.(yaml|yml|json)$/i.test(source)
    ? await renderSpecStructure(text, /\.json$/i.test(source), lines.length, {
      pointer,
      source,
      loadSchema,
      referenceUrl: value => referenceUrl(repoUrl, commit, source, value),
    })
    : null;
  const scenarios = /\.feature$/i.test(source) ? renderGherkinOutline(text, lines.length) : '';
  const code = lines.slice(0, maxLines).map((line, i) => {
    const anchor = 'L' + (i + 1);
    const shown = structure?.targetLine === i + 1 ? `<span id="definition">${escape(line)}</span>` : escape(line);
    return `<span id="${anchor}"><a aria-label="${i + 1} 行目" href="#${anchor}" class="source-line-number muted">${i + 1}</a>  ${shown}</span>`;
  }).join('\n');
  return `${links ? `<nav aria-label="参照ファイル"><h2>参照ファイル</h2><ul>${links}</ul></nav>` : ''}
    ${structure?.html ?? ''}${scenarios}<pre class="code source-code"><code>${code}</code></pre>
    ${lines.length > maxLines ? '<p>最初の10,000行を表示しています。全体は Raw で確認できます。</p>' : ''}`;
}
