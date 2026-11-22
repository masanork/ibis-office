// SPDX-License-Identifier: AGPL-3.0-only
import { isAlias, isMap, isNode, isScalar, type YAMLMap } from 'yaml';
import { escape } from '../vendor/mikaki-oidc';
import { openApiOperations, openApiVersion } from './spec-navigation';

export function renderOpenApiDiagnostics(root: unknown, location: (offset: number) => number): string {
  if (!isMap(root) || !root.has('openapi')) return '';
  const maxDiagnostics = 50;
  const issues: { line: number; message: string }[] = [];
  function report(node: unknown, fallback: unknown, message: string): void {
    if (issues.length >= maxDiagnostics) return;
    const offset = (isNode(node) ? node.range?.[0] : undefined) ?? (isNode(fallback) ? fallback.range?.[0] : undefined) ?? 0;
    issues.push({ line: location(offset), message });
  }
  function stringField(map: YAMLMap, field: string, context: string, required = false): string | null {
    const value = map.get(field, true);
    if (!required && !map.has(field)) return null;
    if (isAlias(value)) return null;
    if (isScalar(value) && typeof value.value === 'string') return value.value;
    report(value, map, context + field + ' は文字列で指定してください。');
    return null;
  }
  function mapField(map: YAMLMap, field: string, required = false): YAMLMap | null {
    const value = map.get(field, true);
    if (!required && !map.has(field)) return null;
    if (isAlias(value)) return null;
    if (isMap(value)) return value;
    report(value, map, field + ' はマッピングで指定してください。');
    return null;
  }

  const rawVersion = root.get('openapi');
  const version = openApiVersion(rawVersion);
  if (!version) {
    report(root.get('openapi', true), root, 'この基本チェックは openapi に 3.0.x、3.1.x、3.2.x のバージョン文字列がある文書を対象とします。');
  } else {
    const info = mapField(root, 'info', true);
    if (info) {
      stringField(info, 'title', 'info.', true);
      stringField(info, 'version', 'info.', true);
    }
    const requiresPaths = version.startsWith('3.0.');
    const paths = mapField(root, 'paths', requiresPaths);
    mapField(root, 'components');
    if (!requiresPaths) {
      mapField(root, 'webhooks');
      if (!['paths', 'components', 'webhooks'].some(field => root.has(field))) {
        report(root, root, 'paths、components、webhooks のいずれかが必要です。');
      }
    }

    if (paths) {
      for (const { key, value } of paths.items) {
        if (!isScalar(key) || typeof key.value !== 'string' || key.value.startsWith('x-')) continue;
        if (!key.value.startsWith('/')) report(key, paths, 'パス名は / から始めてください。');
        else if (!isMap(value) && !isAlias(value)) report(value, key, 'パスの定義はマッピングで指定してください。');
      }
      const operationIds = new Set<string>();
      for (const { key, value } of openApiOperations(paths, version)) {
        if (issues.length >= maxDiagnostics) break;
        if (isAlias(value)) continue;
        if (!isMap(value)) {
          report(value, key, '操作の定義はマッピングで指定してください。');
          continue;
        }
        const id = stringField(value, 'operationId', '');
        if (id !== null) {
          if (operationIds.has(id)) report(value.get('operationId', true), key, 'operationId が同じ文書の別の操作と重複しています。');
          operationIds.add(id);
        }
        const responses = mapField(value, 'responses', requiresPaths);
        if (responses) {
          const hasStatus = responses.items.some(({ key }) => isScalar(key) &&
            typeof key.value === 'string' && /^(?:default|[1-5](?:\d\d|XX))$/.test(key.value));
          if (!hasStatus) report(responses, key, 'responses に応答コードまたは default を指定してください。');
        }
      }
    }
  }

  const items = issues.map(({ line, message }) => `<li><a href="#L${line}">${line} 行目</a> · ${escape(message)}</li>`).join('');
  return `<aside class="source-structure" aria-label="OpenAPI 基本チェック"><h2>OpenAPI 基本チェック</h2>
    ${items ? `<ul>${items}</ul>` : '<p>確認対象の項目に問題は見つかりませんでした。</p>'}
    ${issues.length === maxDiagnostics ? '<p class="muted">診断は最大50件まで表示します。</p>' : ''}</aside>`;
}
