// SPDX-License-Identifier: AGPL-3.0-only
import { isMap, isNode, isScalar, isSeq, type YAMLMap } from 'yaml';
import { escape } from '../vendor/mikaki-oidc';

export function pointerTarget(root: unknown, pointer: string): { node: unknown; position: unknown } | null {
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
  return { node, position };
}

export function pointerLine(root: unknown, pointer: string, location: (offset: number) => number): number | null {
  const position = pointerTarget(root, pointer)?.position;
  return isNode(position) && position.range ? location(position.range[0]) : null;
}

const HTTP_METHODS = new Set(['get', 'put', 'post', 'delete', 'options', 'head', 'patch', 'trace']);

export function openApiVersion(value: unknown): string | null {
  return typeof value === 'string' && /^3\.[012]\.\d+$/.test(value) ? value : null;
}

export function* openApiOperations(paths: YAMLMap, version: string) {
  const additionalMethods = version.startsWith('3.2.');
  for (const path of paths.items) {
    if (!isScalar(path.key) || typeof path.key.value !== 'string' || !path.key.value.startsWith('/') || !isMap(path.value)) continue;
    for (const { key, value } of path.value.items) {
      if (!isScalar(key) || typeof key.value !== 'string' || !key.range) continue;
      if (HTTP_METHODS.has(key.value) || (additionalMethods && key.value === 'query')) {
        yield { path: path.key.value, method: key.value.toUpperCase(), key, value };
      } else if (additionalMethods && key.value === 'additionalOperations' && isMap(value)) {
        for (const custom of value.items) {
          if (isScalar(custom.key) && typeof custom.key.value === 'string' && custom.key.range) {
            yield { path: path.key.value, method: custom.key.value, key: custom.key, value: custom.value };
          }
        }
      }
    }
  }
}

export function renderOpenApiOperations(root: unknown, location: (offset: number) => number): string {
  if (!isMap(root)) return '';
  const version = openApiVersion(root.get('openapi'));
  const paths = root.get('paths', true);
  if (!version || !isMap(paths)) return '';
  const operations: string[] = [];
  for (const { path, method, value, key } of openApiOperations(paths, version)) {
    if (!isMap(value) || !key.range) continue;
    const label = value.get('summary') ?? value.get('operationId');
    const detail = typeof label === 'string' ? ` <span class="muted">${escape(label.slice(0, 160))}</span>` : '';
    operations.push(`<li><a href="#L${location(key.range[0])}"><code>${escape(method)} ${escape(path)}</code></a>${detail}</li>`);
    if (operations.length >= 100) break;
  }
  if (!operations.length) return '';
  return `<nav class="source-structure" aria-label="OpenAPI 操作"><h2>OpenAPI 操作</h2><ul>${operations.join('')}</ul>
    ${operations.length === 100 ? '<p class="muted">操作は最大100件まで表示します。</p>' : ''}</nav>`;
}
