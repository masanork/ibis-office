// SPDX-License-Identifier: AGPL-3.0-only
import { LineCounter, isMap, isScalar, isSeq, parseAllDocuments } from 'yaml';
import { escape } from '../vendor/mikaki-oidc';
import { parseGherkinFeature } from './spec-gherkin';
import { parseTestLedger } from './spec-ledger';
import { openApiOperations, openApiVersion } from './spec-navigation';

export interface Requirement {
  id: string;
  line: number;
}

export function requirementHeading(line: string, number: number): Requirement | null {
  const match = /^ {0,3}#{1,6}[ \t]+(REQ-[A-Za-z0-9][A-Za-z0-9._-]{0,59})(?=[:： \t]|$)/.exec(line);
  return match ? { id: match[1], line: number } : null;
}

type Feature = NonNullable<ReturnType<typeof parseGherkinFeature>>;
interface TaggedEntry {
  line: number;
  label: string;
  ids: readonly string[];
}

function* taggedCases(children: Feature['children'], inherited: readonly string[]):
  Generator<TaggedEntry> {
  for (const child of children) {
    if (child.rule) {
      yield* taggedCases(child.rule.children, [...inherited, ...child.rule.tags.map(tag => tag.name)]);
    } else if (child.scenario) {
      const scenario = child.scenario;
      yield {
        line: scenario.location.line, label: scenario.name,
        ids: [...inherited, ...scenario.tags.map(tag => tag.name)].map(tag => tag.slice(1)),
      };
      for (const example of scenario.examples) {
        if (!example.tags.length) continue;
        yield {
          line: example.location.line,
          label: scenario.name + ' / ' + (example.name || example.keyword),
          ids: example.tags.map(tag => tag.name.slice(1)),
        };
      }
    }
  }
}

function* taggedOperations(source: string, path: string): Generator<TaggedEntry> {
  const input = source.replace(/\r\n?/g, '\n');
  if (/\.json$/i.test(path)) JSON.parse(input.replace(/^\uFEFF/, ''));
  const lines = new LineCounter();
  const documents = parseAllDocuments(input, { lineCounter: lines, prettyErrors: false, uniqueKeys: false });
  if (documents.some(document => document.errors.length)) throw new Error('Invalid specification source');
  if (documents.length !== 1 || !isMap(documents[0].contents)) return;
  const root = documents[0].contents;
  const versionNode = root.get('openapi', true);
  const version = openApiVersion(isScalar(versionNode) ? versionNode.value : null);
  const paths = root.get('paths', true);
  if (!version || !isMap(paths)) return;
  for (const { path, method, key, value } of openApiOperations(paths, version)) {
    if (!isMap(value) || !key.range) continue;
    const tags = value.get('tags', true);
    if (!isSeq(tags)) continue;
    const ids = tags.items.flatMap(tag => isScalar(tag) && typeof tag.value === 'string' ? [tag.value] : []);
    const id = value.get('operationId');
    const label = method + ' ' + path + (typeof id === 'string' ? ' · ' + id : '');
    yield { line: lines.linePos(key.range[0]).line, label, ids };
  }
}

function* acceptanceTests(source: string): Generator<TaggedEntry> {
  const feature = parseGherkinFeature(source);
  if (!feature) throw new Error('No Gherkin feature');
  yield* taggedCases(feature.children, feature.tags.map(tag => tag.name));
}

export async function renderRequirementTrace(
  requirements: Requirement[], featurePaths: ReadonlySet<string>, apiPaths: ReadonlySet<string>, ledgerPaths: ReadonlySet<string>,
  loadSource: (path: string) => Promise<string | null>, sourceUrl: (path: string) => string,
): Promise<string> {
  if (!requirements.length || (!featurePaths.size && !apiPaths.size && !ledgerPaths.size)) return '';
  const shown = requirements.slice(0, 100);
  const matches = new Map(shown.map(({ id }) => [id, [] as string[]]));
  const tests = [...featurePaths].slice(0, 4);
  const specs = [...apiPaths].slice(0, 4);
  const ledgers = [...ledgerPaths].slice(0, 4);
  let unreadable = 0;
  let truncated = shown.length < requirements.length || tests.length < featurePaths.size || specs.length < apiPaths.size || ledgers.length < ledgerPaths.size;
  for (const path of [...tests, ...specs, ...ledgers]) {
    try {
      const source = await loadSource(path);
      if (source === null || source.length > 128 * 1024) { unreadable++; continue; }
      const api = apiPaths.has(path);
      const ledger = ledgerPaths.has(path);
      const entries = api ? taggedOperations(source, path) : ledger ? parseTestLedger(source, path) : acceptanceTests(source);
      let count = 0;
      const url = sourceUrl(path);
      for (const entry of entries) {
        if (count++ === 200 || entry.line > 10_000) { truncated = true; break; }
        for (const id of new Set(entry.ids)) {
          const links = matches.get(id);
          if (!links) continue;
          if (links.length >= 20) { truncated = true; continue; }
          links.push(`<li><a href="${escape(url)}#L${entry.line}">${escape(entry.label.slice(0, 160) || path)}</a> <span class="muted">${api ? 'API · ' : ledger ? '試験台帳 · ' : ''}${escape(path)}</span></li>`);
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
  return `<nav class="source-structure" aria-label="要求の対応表"><h2>要求の対応表</h2>
    <p class="muted">このMarkdownが直接リンクするOpenAPI操作・Gherkinシナリオ・CSV／TSV試験台帳を照合しています。</p><ul>${items}</ul>
    ${unreadable ? `<p class="muted">${unreadable} 件の参照ファイルを確認できませんでした。</p>` : ''}
    ${truncated ? '<p class="muted">表示上限に達したため、一部を省略しています。</p>' : ''}</nav>`;
}
