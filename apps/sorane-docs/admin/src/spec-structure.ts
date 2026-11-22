// SPDX-License-Identifier: AGPL-3.0-only
import { LineCounter, isAlias, isMap, isNode, isScalar, isSeq, parseAllDocuments } from 'yaml';
import { escape } from '../vendor/mikaki-oidc';
import { pointerLine, renderOpenApiOperations } from './spec-navigation';
import { renderOpenApiDiagnostics } from './spec-diagnostics';
import { renderOpenApiExamples } from './spec-examples';

const MAX_ENTRIES = 1000;
const MAX_DEPTH = 16;
const MAX_PARSE_LENGTH = 128 * 1024;

function abbreviated(text: string): string {
  return text.length > 120 ? text.slice(0, 120) + '…' : text;
}

interface StructureOptions {
  pointer: string | null;
  referenceUrl(value: string): string | null;
}

export function renderSpecStructure(text: string, json: boolean, maxLine: number, options: StructureOptions) {
  const notice = (message: string) => '<p class="muted">' + message + '</p>';
  const message = (html: string) => ({ html, targetLine: null });

  if (text.length > MAX_PARSE_LENGTH || maxLine > 10_000) {
    return message(notice('ファイルが構造表示の上限を超えています。原文は引き続き参照できます。'));
  }

  // Match the source pane's line endings and leave the raw blob untouched.
  const input = text.replace(/\r\n?/g, '\n');
  const lineCounter = new LineCounter();
  let documents: ReturnType<typeof parseAllDocuments>;
  try {
    // Keep each key occurrence in the source tree without comparing all pairs.
    documents = parseAllDocuments(input, { lineCounter, prettyErrors: false, uniqueKeys: false });
  } catch (error) {
    if (!(error instanceof RangeError)) throw error;
    return message(notice('入れ子が深いため構造を表示できません。原文は引き続き参照できます。'));
  }
  const error = documents.find(document => document.errors.length > 0)?.errors[0];
  const location = (offset: number) => lineCounter.linePos(offset).line;
  if (error) {
    const line = location(error.pos[0]);
    const link = line >= 1 && line <= maxLine ? `<a href="#L${line}">${line} 行目</a> · ` : '';
    return message(`<aside aria-label="構文エラー"><p>構造を表示できません。 ${link}${escape(abbreviated(error.message))}</p></aside>`);
  }
  if (json) {
    try {
      JSON.parse(input.replace(/^\uFEFF/, ''));
    } catch {
      return message(notice('JSONの構文を確認してください。'));
    }
  }

  const root = documents.length === 1 ? documents[0].contents : null;
  const targetLine = options.pointer === null || !root ? null : pointerLine(root, options.pointer, location);
  const operations = renderOpenApiOperations(root, location);
  const diagnostics = renderOpenApiDiagnostics(root, location);
  const examples = renderOpenApiExamples(root, location);
  const missing = options.pointer !== null && targetLine === null ? notice('指定された参照先を特定できませんでした。') : '';
  let remaining = MAX_ENTRIES;
  let truncated = false;
  const tree: string[] = [];
  function entry(label: string, node: unknown, depth: number, key?: unknown, localBase = true): void {
    if (remaining === 0 || depth > MAX_DEPTH) {
      truncated = true;
      return;
    }
    remaining -= 1;
    const positionNode = isNode(key) ? key : isNode(node) ? node : null;
    const line = location(positionNode?.range?.[0] ?? 0);
    const safeLabel = escape(abbreviated(label));
    const title = line >= 1 && line <= maxLine
      ? `<a href="#L${line}" aria-label="${safeLabel}（${line} 行目）">${safeLabel}</a>`
      : safeLabel;

    if (isMap(node) || isSeq(node)) {
      const count = node.items.length;
      if (count === 0) {
        tree.push(`<li>${title} <span class="muted">${isMap(node) ? '{}' : '[]'}</span></li>`);
        return;
      }
      tree.push(`<li><details${depth === 0 ? ' open' : ''}><summary>${title} <span class="muted">${count} 項目</span></summary><ul>`);
      if (isMap(node)) {
        const fileRelative = localBase && !node.has('$id') && !node.has('$self');
        for (const pair of node.items) {
          if (remaining === 0 || depth === MAX_DEPTH) { truncated = true; break; }
          const name = isScalar(pair.key) ? pair.key.source ?? String(pair.key.value) : '複合キー';
          entry(name || '（空のキー）', pair.value, depth + 1, pair.key, fileRelative);
        }
      } else {
        for (let i = 0; i < count; i++) {
          if (remaining === 0 || depth === MAX_DEPTH) { truncated = true; break; }
          entry('[' + i + ']', node.items[i], depth + 1, undefined, localBase);
        }
      }
      tree.push('</ul></details></li>');
    } else {
      // Alias nodes link to the occurrence. Expansion is unnecessary for navigation.
      const value = isAlias(node) ? '*' + node.source : isScalar(node) ? node.source ?? String(node.value) : 'null';
      const url = localBase && isScalar(key) && key.value === '$ref' && isScalar(node) && typeof node.value === 'string'
        ? options.referenceUrl(node.value) : null;
      const shown = `<code class="muted">${escape(abbreviated(value))}</code>`;
      const reference = url ? `<a href="${escape(url)}" aria-label="参照先を開く">${shown}</a>` : shown;
      tree.push(`<li>${title} ${reference}</li>`);
    }
  }

  for (const [index, document] of documents.entries()) {
    if (remaining === 0) { truncated = true; break; }
    if (document.contents) {
      entry(documents.length === 1 ? 'ルート' : `ドキュメント ${index + 1}`, document.contents, 0, undefined, documents.length === 1);
    }
  }
  if (!tree.length) return { html: missing, targetLine };
  const html = `${missing}${operations}${diagnostics}${examples}<nav class="source-structure" aria-label="構造ツリー"><h2>構造ツリー</h2><ul>${tree.join('')}</ul>
    ${truncated ? notice('構造ツリーの一部を表示しています。') : ''}</nav>`;
  return { html, targetLine };
}
