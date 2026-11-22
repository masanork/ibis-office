// SPDX-License-Identifier: AGPL-3.0-only
import { escape } from '../vendor/mikaki-oidc';
import { parseGherkinFeature } from './spec-gherkin';

export interface Requirement {
  id: string;
  line: number;
}

export function requirementHeading(line: string, number: number): Requirement | null {
  const match = /^ {0,3}#{1,6}[ \t]+(REQ-[A-Za-z0-9][A-Za-z0-9._-]{0,59})(?=[:： \t]|$)/.exec(line);
  return match ? { id: match[1], line: number } : null;
}

type Feature = NonNullable<ReturnType<typeof parseGherkinFeature>>;

function* taggedCases(children: Feature['children'], inherited: readonly string[]):
  Generator<{ line: number; label: string; tags: readonly string[] }> {
  for (const child of children) {
    if (child.rule) {
      yield* taggedCases(child.rule.children, [...inherited, ...child.rule.tags.map(tag => tag.name)]);
    } else if (child.scenario) {
      const scenario = child.scenario;
      yield {
        line: scenario.location.line, label: scenario.name,
        tags: [...inherited, ...scenario.tags.map(tag => tag.name)],
      };
      for (const example of scenario.examples) {
        if (!example.tags.length) continue;
        yield {
          line: example.location.line,
          label: scenario.name + ' / ' + (example.name || example.keyword),
          tags: example.tags.map(tag => tag.name),
        };
      }
    }
  }
}

export async function renderRequirementTrace(
  requirements: Requirement[], featurePaths: ReadonlySet<string>,
  loadSource: (path: string) => Promise<string | null>, sourceUrl: (path: string) => string,
): Promise<string> {
  if (!requirements.length || !featurePaths.size) return '';
  const shown = requirements.slice(0, 100);
  const matches = new Map(shown.map(({ id }) => [id, [] as string[]]));
  const paths = [...featurePaths].slice(0, 4);
  let unreadable = 0;
  let truncated = shown.length < requirements.length || paths.length < featurePaths.size;
  for (const path of paths) {
    try {
      const source = await loadSource(path);
      if (source === null || source.length > 128 * 1024) { unreadable++; continue; }
      const feature = parseGherkinFeature(source);
      if (!feature) { unreadable++; continue; }
      let count = 0;
      const url = sourceUrl(path);
      for (const entry of taggedCases(feature.children, feature.tags.map(tag => tag.name))) {
        if (count++ === 200 || entry.line > 10_000) { truncated = true; break; }
        for (const tag of new Set(entry.tags)) {
          if (!tag.startsWith('@')) continue;
          const links = matches.get(tag.slice(1));
          if (!links) continue;
          if (links.length >= 20) { truncated = true; continue; }
          links.push(`<li><a href="${escape(url)}#L${entry.line}">${escape(entry.label.slice(0, 160) || path)}</a> <span class="muted">${escape(path)}</span></li>`);
        }
      }
    } catch {
      unreadable++;
    }
  }
  const items = shown.map(({ id, line }) => {
    const links = matches.get(id) ?? [];
    return `<li><a href="#L${line}">${escape(id)}</a>
      ${links.length ? `<ul>${links.join('')}</ul>` : ' <span class="muted">確認できたリンク先に一致なし</span>'}</li>`;
  }).join('');
  return `<nav class="source-structure" aria-label="要求と受け入れテスト"><h2>要求と受け入れテスト</h2>
    <p class="muted">このMarkdownが直接リンクする .feature ファイルのタグを照合しています。</p><ul>${items}</ul>
    ${unreadable ? `<p class="muted">${unreadable} 件の .feature を確認できませんでした。</p>` : ''}
    ${truncated ? '<p class="muted">表示上限に達したため、一部を省略しています。</p>' : ''}</nav>`;
}
