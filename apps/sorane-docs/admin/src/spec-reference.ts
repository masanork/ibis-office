// SPDX-License-Identifier: AGPL-3.0-only
import { normalizeRepoPath } from './git-pack';

export function resolveRelativePath(source: string, pathname: string): string | null {
  if (/^(?:[a-z][a-z0-9+.-]*:|\/|#|\?)/i.test(pathname) || pathname.includes('?')) return null;
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
  return normalizeRepoPath(segments.join('/'));
}

export function schemaReference(source: string, value: string) {
  if (!value || value.length > 2048) return null;
  const hash = value.indexOf('#');
  const pathname = hash < 0 ? value : value.slice(0, hash);
  const path = pathname ? resolveRelativePath(source, pathname) : source;
  if (!path || !/\.(yaml|yml|json)$/i.test(path)) return null;
  try {
    const pointer = hash < 0 ? '' : decodeURIComponent(value.slice(hash + 1));
    if (pointer && (!pointer.startsWith('/') || /~(?![01])/.test(pointer))) return null;
    return { path, pointer };
  } catch {
    return null;
  }
}
