// SPDX-License-Identifier: AGPL-3.0-only
import { AstBuilder, Errors, GherkinClassicTokenMatcher, Parser } from '@cucumber/gherkin';
import { escape } from '../vendor/mikaki-oidc';

export function renderGherkinOutline(text: string, maxLine: number): string {
  if (text.length > 128 * 1024 || maxLine > 10_000) {
    return '<p class="muted">ファイルがシナリオ一覧の上限を超えています。原文は引き続き参照できます。</p>';
  }
  const link = (line: number, label: string) =>
    line >= 1 && line <= maxLine ? `<a href="#L${line}">${escape(label)}</a>` : escape(label);
  const tags = (values: readonly { name: string; location: { line: number } }[]) =>
    values.slice(0, 20).map(tag => ` <span class="badge">${link(tag.location.line, tag.name.slice(0, 120))}</span>`).join('');
  let id = 0;
  const parser = new Parser(new AstBuilder(() => String(++id)), new GherkinClassicTokenMatcher());
  let feature: ReturnType<typeof parser.parse>['feature'];
  try {
    feature = parser.parse(text.replace(/^\uFEFF/, '').replace(/\r\n?/g, '\n')).feature;
  } catch (error) {
    if (!(error instanceof Errors.GherkinException)) throw error;
    const first = error.errors[0] ?? error;
    const line = first instanceof Errors.GherkinException ? first.location?.line : undefined;
    const where = line ? link(line, `${line} 行目`) + ' · ' : '';
    return `<aside aria-label="Gherkin 構文エラー"><p>${where}${escape(first.message.slice(0, 220))}</p></aside>`;
  }
  if (!feature) return '';
  const limit = 200;
  let count = 0;
  let truncated = false;
  function scenarios(children: NonNullable<ReturnType<typeof parser.parse>['feature']>['children']): string {
    const items: string[] = [];
    for (const child of children) {
      if (!child.scenario && !child.rule) continue;
      if (count >= limit) { truncated = true; break; }
      count++;
      if (child.rule) {
        const rule = child.rule;
        items.push(`<li>${link(rule.location.line, rule.keyword + ': ' + rule.name)}${tags(rule.tags)}${scenarios(rule.children)}</li>`);
      } else if (child.scenario) {
        const scenario = child.scenario;
        const examples = scenario.examples.slice(0, limit - count).map(example => {
          count++;
          return `<li>${link(example.location.line, example.keyword + (example.name ? ': ' + example.name : ''))}
            <span class="muted">${example.tableBody.length} 行</span>${tags(example.tags)}</li>`;
        });
        if (examples.length < scenario.examples.length) truncated = true;
        items.push(`<li>${link(scenario.location.line, scenario.keyword + ': ' + scenario.name)}${tags(scenario.tags)}
          ${examples.length ? `<ul>${examples.join('')}</ul>` : ''}</li>`);
      }
    }
    return items.length ? `<ul>${items.join('')}</ul>` : '';
  }

  const outline = scenarios(feature.children);
  return `<nav class="source-structure" aria-label="Gherkin シナリオ"><h2>Gherkin シナリオ</h2>
    <p>${link(feature.location.line, feature.keyword + ': ' + feature.name)}${tags(feature.tags)}</p>${outline}
    ${truncated ? '<p class="muted">シナリオ一覧の一部を表示しています。</p>' : ''}</nav>`;
}
