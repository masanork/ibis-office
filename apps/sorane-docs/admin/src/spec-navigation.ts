// SPDX-License-Identifier: AGPL-3.0-only
import { isMap, isNode, isScalar, isSeq } from 'yaml';
import { escape } from '../vendor/mikaki-oidc';

export function pointerLine(root: unknown, pointer: string, location: (offset: number) => number): number | null {
  if (pointer !== '' && (!pointer.startsWith('/') || /~(?![01])/.test(pointer))) return null;
  const tokens = pointer.split('/').slice(1);
  if (tokens.length > 64) return null;
  let node = root;
  let position = root;
  for (const token of tokens) {
    const key = token.replace(/~1/g, '/').replace(/~0/g, '~');
    if (isMap(node)) {
      const pairs = node.items.filter(pair => isScalar(pair.key) && pair.key.value === key);
      if (pairs.length !== 1) return null;
      node = pairs[0].value;
      position = pairs[0].key;
    } else if (isSeq(node) && /^(?:0|[1-9]\d*)$/.test(key) && Number(key) < node.items.length) {
      node = node.items[Number(key)];
      position = node;
    } else {
      return null;
    }
  }
  return isNode(position) && position.range ? location(position.range[0]) : null;
}

const HTTP_METHODS = new Set(['get', 'put', 'post', 'delete', 'options', 'head', 'patch', 'trace']);

export function renderOpenApiOperations(root: unknown, location: (offset: number) => number): string {
  if (!isMap(root)) return '';
  const version = root.get('openapi');
  if (typeof version !== 'string' || !/^3\.[012]\.\d+$/.test(version)) return '';
  const paths = root.get('paths', true);
  if (!isMap(paths)) return '';
  const supportsAdditionalMethods = version.startsWith('3.2.');
  const operations: string[] = [];
  function addOperation(path: string, method: string, value: unknown, offset: number): void {
    if (!isMap(value)) return;
    const label = value.get('summary') ?? value.get('operationId');
    const detail = typeof label === 'string' ? ` <span class="muted">${escape(label.slice(0, 160))}</span>` : '';
    operations.push(`<li><a href="#L${location(offset)}"><code>${escape(method)} ${escape(path)}</code></a>${detail}</li>`);
  }

  for (const path of paths.items) {
    if (operations.length >= 100) break;
    if (!isScalar(path.key) || typeof path.key.value !== 'string' || !path.key.value.startsWith('/') || !isMap(path.value)) continue;
    for (const { key, value } of path.value.items) {
      if (operations.length >= 100) break;
      if (!isScalar(key) || typeof key.value !== 'string' || !key.range) continue;
      if (HTTP_METHODS.has(key.value) || (supportsAdditionalMethods && key.value === 'query')) {
        addOperation(path.key.value, key.value.toUpperCase(), value, key.range[0]);
      } else if (supportsAdditionalMethods && key.value === 'additionalOperations' && isMap(value)) {
        for (const custom of value.items) {
          if (operations.length >= 100) break;
          if (isScalar(custom.key) && typeof custom.key.value === 'string' && custom.key.range) {
            addOperation(path.key.value, custom.key.value, custom.value, custom.key.range[0]);
          }
        }
      }
    }
  }
  if (!operations.length) return '';
  return `<nav class="source-structure" aria-label="OpenAPI 操作"><h2>OpenAPI 操作</h2><ul>${operations.join('')}</ul>
    ${operations.length === 100 ? '<p class="muted">操作は最大100件まで表示します。</p>' : ''}</nav>`;
}
