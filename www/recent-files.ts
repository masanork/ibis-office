// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
// Browser-only recent-files history. Names, handles and GitHub paths are
// kept in a dedicated IndexedDB; file contents are never stored.

import {
  fileFromHandle,
  pickLocalFile,
  queryHandlePermission,
  setCurrentHandle,
  type PickedLocalFile,
} from './local-file.js';

export const RECENTS_DB_NAME = 'ibisoffice-recents';
export const RECENTS_DB_VERSION = 1;
export const RECENTS_STORE = 'entries';
export const RECENTS_STORE_LIMIT = 20;
export const RECENTS_SHOW_LIMIT = 5;

export type RecentSource = 'local' | 'github' | 'sorane-cloud';

export type RecentGithub = {
  owner: string;
  repo: string;
  ref: string;
  path: string;
};

export type RecentRecord = {
  id: string;
  name: string;
  kind: string;
  openedAt: number;
  source: RecentSource;
  handle?: FileSystemFileHandle;
  github?: RecentGithub;
};

export type StartViewRecent = {
  id: string;
  title: string;
  subtitle: string;
  name: string;
  badge: string;
  action: string;
};

export type RecentOpenInput = {
  source: RecentSource;
  name: string;
  handle?: FileSystemFileHandle;
  github?: RecentGithub;
  openedAt?: number;
};

/** File kind shown as a badge (docx, xlsx, md, …). */
export function kindFromName(name: string): string {
  const at = name.lastIndexOf('.');
  if (at < 0 || at === name.length - 1) return 'file';
  const ext = name.slice(at + 1).toLowerCase();
  if (ext === 'markdown') return 'md';
  return ext.slice(0, 6);
}

/** Stable id used to dedupe. Local files of the same name share one slot. */
export function recentId(input: Pick<RecentOpenInput, 'source' | 'name' | 'github'>): string {
  if (input.source === 'github' && input.github) {
    const g = input.github;
    return `github:${g.owner}/${g.repo}@${g.ref}:${g.path}`;
  }
  if (input.source === 'sorane-cloud') {
    return `sorane:${input.name}`;
  }
  return `local:${input.name}`;
}

/** Drop an accidental `bytes` field so contents never land in IndexedDB. */
export function persistableRecord(entry: RecentRecord): RecentRecord {
  const next: RecentRecord = {
    id: entry.id,
    name: entry.name,
    kind: entry.kind,
    openedAt: entry.openedAt,
    source: entry.source,
  };
  if (entry.handle) next.handle = entry.handle;
  if (entry.github) next.github = { ...entry.github };
  return next;
}

export function recordFromInput(input: RecentOpenInput): RecentRecord {
  return persistableRecord({
    id: recentId(input),
    name: input.name,
    kind: kindFromName(input.name),
    openedAt: input.openedAt ?? Date.now(),
    source: input.source,
    handle: input.handle,
    github: input.github,
  });
}

/**
 * Insert or replace by id, most recently opened first, capped at `limit`.
 * Same-name local files share an id; a new handle overwrites the previous one.
 */
export function upsertRecent(
  list: RecentRecord[],
  incoming: RecentRecord,
  limit = RECENTS_STORE_LIMIT,
): RecentRecord[] {
  const next = list.filter((entry) => entry.id !== incoming.id);
  next.unshift(persistableRecord(incoming));
  return next.slice(0, Math.max(0, limit));
}

export function visibleRecents(list: RecentRecord[], limit = RECENTS_SHOW_LIMIT): RecentRecord[] {
  return list.slice(0, Math.max(0, limit));
}

export function removeRecentById(list: RecentRecord[], id: string): RecentRecord[] {
  return list.filter((entry) => entry.id !== id);
}

export function formatRelativeTime(openedAt: number, now: number, locale: 'ja' | 'en' = 'ja'): string {
  const sec = Math.max(0, Math.floor((now - openedAt) / 1000));
  if (locale === 'ja') {
    if (sec < 60) return 'たった今';
    const min = Math.floor(sec / 60);
    if (min < 60) return `${min}分前`;
    const hours = Math.floor(min / 60);
    if (hours < 24) return `${hours}時間前`;
    const days = Math.floor(hours / 24);
    if (days === 1) return '昨日';
    if (days < 7) return `${days}日前`;
    const d = new Date(openedAt);
    return `${d.getMonth() + 1}月${d.getDate()}日`;
  }
  if (sec < 60) return 'just now';
  const min = Math.floor(sec / 60);
  if (min < 60) return min === 1 ? '1 minute ago' : `${min} minutes ago`;
  const hours = Math.floor(min / 60);
  if (hours < 24) return hours === 1 ? '1 hour ago' : `${hours} hours ago`;
  const days = Math.floor(hours / 24);
  if (days === 1) return 'yesterday';
  if (days < 7) return `${days} days ago`;
  return new Date(openedAt).toLocaleDateString('en');
}

export function canReopenDirectly(entry: RecentRecord): boolean {
  return Boolean(entry.handle) || Boolean(entry.github);
}

export function toStartViewRecent(
  entry: RecentRecord,
  now: number,
  locale: 'ja' | 'en' = 'ja',
): StartViewRecent {
  const time = formatRelativeTime(entry.openedAt, now, locale);
  let subtitle = time;
  if (entry.source === 'github' && entry.github) {
    subtitle = `${entry.github.owner}/${entry.github.repo} · ${time}`;
  } else if (!entry.handle) {
    const reopen = locale === 'ja' ? '再度開く' : 'Open again';
    subtitle = `${reopen} · ${time}`;
  }
  return {
    id: entry.id,
    title: entry.name,
    subtitle,
    name: entry.name,
    badge: entry.kind.toUpperCase(),
    action: `open-recent:${entry.id}`,
  };
}

type RecentsIdb = { open: (name: string, version: number) => IDBOpenDBRequest };

function recentsDb(api: RecentsIdb = indexedDB): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const open = api.open(RECENTS_DB_NAME, RECENTS_DB_VERSION);
    open.onupgradeneeded = () => {
      const db = open.result;
      if (!db.objectStoreNames.contains(RECENTS_STORE)) {
        db.createObjectStore(RECENTS_STORE);
      }
    };
    open.onerror = () => reject(open.error ?? new Error('failed to open recents db'));
    open.onsuccess = () => resolve(open.result);
  });
}

const KEY = 'list';

export async function loadRecents(api: RecentsIdb = indexedDB): Promise<RecentRecord[]> {
  try {
    const db = await recentsDb(api);
    const loaded = await new Promise<unknown>((resolve, reject) => {
      const request = db.transaction(RECENTS_STORE, 'readonly').objectStore(RECENTS_STORE).get(KEY);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    if (!Array.isArray(loaded)) return [];
    return loaded.map((row) => persistableRecord(row as RecentRecord)).filter((row) => row.id && row.name);
  } catch {
    return [];
  }
}

export async function saveRecents(list: RecentRecord[], api: RecentsIdb = indexedDB): Promise<void> {
  const db = await recentsDb(api);
  const stored = list.slice(0, RECENTS_STORE_LIMIT).map(persistableRecord);
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(RECENTS_STORE, 'readwrite');
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.objectStore(RECENTS_STORE).put(stored, KEY);
  });
}

export async function recordOpened(input: RecentOpenInput, api: RecentsIdb = indexedDB): Promise<RecentRecord[]> {
  const list = upsertRecent(await loadRecents(api), recordFromInput(input));
  await saveRecents(list, api);
  return list;
}

export async function removeOpened(id: string, api: RecentsIdb = indexedDB): Promise<RecentRecord[]> {
  const list = removeRecentById(await loadRecents(api), id);
  await saveRecents(list, api);
  return list;
}

export async function clearOpened(api: RecentsIdb = indexedDB): Promise<void> {
  await saveRecents([], api);
}

export async function listStartViewRecents(
  now = Date.now(),
  locale: 'ja' | 'en' = 'ja',
  api: RecentsIdb = indexedDB,
): Promise<StartViewRecent[]> {
  const list = visibleRecents(await loadRecents(api));
  return list.map((entry) => toStartViewRecent(entry, now, locale));
}

export type OpenRecentResult =
  | { mode: 'handle'; file: PickedLocalFile }
  | { mode: 'github'; github: RecentGithub }
  | { mode: 'picker'; file: PickedLocalFile | null }
  | { mode: 'missing' };

/** Reopen a history row: the stored handle, a GitHub path, or the file picker (「再度開く」). */
export async function resolveRecentOpen(
  entry: RecentRecord | undefined,
  pick: () => Promise<PickedLocalFile | null> = () => pickLocalFile({ remember: true }),
): Promise<OpenRecentResult> {
  if (!entry) return { mode: 'missing' };
  if (entry.source === 'github' && entry.github) {
    return { mode: 'github', github: entry.github };
  }
  if (entry.handle) {
    try {
      if (await queryHandlePermission(entry.handle, 'read')) {
        const file = await fileFromHandle(entry.handle);
        setCurrentHandle(entry.handle);
        return { mode: 'handle', file };
      }
    } catch {
      // Permission or a vanished file: fall through to the picker.
    }
  }
  return { mode: 'picker', file: await pick() };
}
