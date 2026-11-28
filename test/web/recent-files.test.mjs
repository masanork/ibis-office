// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  RECENTS_SHOW_LIMIT,
  RECENTS_STORE_LIMIT,
  canReopenDirectly,
  formatRelativeTime,
  kindFromName,
  persistableRecord,
  recentId,
  recordFromInput,
  removeRecentById,
  resolveRecentOpen,
  toStartViewRecent,
  upsertRecent,
  visibleRecents,
} from '../../www/recent-files.js';

test('kind and id classify local, github and cloud entries', () => {
  assert.equal(kindFromName('note.DOCX'), 'docx');
  assert.equal(kindFromName('memo.markdown'), 'md');
  assert.equal(kindFromName('noext'), 'file');
  assert.equal(recentId({ source: 'local', name: '契約書.docx' }), 'local:契約書.docx');
  assert.equal(
    recentId({ source: 'github', name: 'a.md', github: { owner: 'acme', repo: 'docs', ref: 'main', path: 'notes/a.md' } }),
    'github:acme/docs@main:notes/a.md',
  );
  assert.equal(recentId({ source: 'sorane-cloud', name: 'cloud.docx' }), 'sorane:cloud.docx');
});

test('upsert dedupes by id, newest first, and caps the store', () => {
  const t0 = 1_000;
  let list = [];
  list = upsertRecent(list, recordFromInput({ source: 'local', name: 'a.docx', openedAt: t0 }));
  list = upsertRecent(list, recordFromInput({ source: 'local', name: 'b.xlsx', openedAt: t0 + 1 }));
  list = upsertRecent(list, recordFromInput({ source: 'local', name: 'a.docx', openedAt: t0 + 2 }));
  assert.equal(list.length, 2);
  assert.equal(list[0].name, 'a.docx');
  assert.equal(list[0].openedAt, t0 + 2);
  assert.equal(list[1].name, 'b.xlsx');

  for (let i = 0; i < RECENTS_STORE_LIMIT + 5; i++) {
    list = upsertRecent(list, recordFromInput({ source: 'local', name: `f${i}.md`, openedAt: t0 + 10 + i }));
  }
  assert.equal(list.length, RECENTS_STORE_LIMIT);
  assert.equal(list[0].name, `f${RECENTS_STORE_LIMIT + 4}.md`);
  assert.equal(visibleRecents(list).length, RECENTS_SHOW_LIMIT);
  assert.equal(visibleRecents(list)[0].name, list[0].name);
});

test('persistableRecord drops file contents if they were attached', () => {
  const dirty = recordFromInput({ source: 'local', name: 'a.md', openedAt: 1 });
  dirty.bytes = new Uint8Array([1, 2, 3]);
  const clean = persistableRecord(dirty);
  assert.equal('bytes' in clean, false);
  assert.equal(clean.name, 'a.md');
});

test('remove and relative time labels', () => {
  const now = Date.UTC(2026, 10, 26, 12, 0, 0);
  const list = removeRecentById([
    recordFromInput({ source: 'local', name: 'keep.md', openedAt: now }),
    recordFromInput({ source: 'local', name: 'gone.md', openedAt: now }),
  ], 'local:gone.md');
  assert.deepEqual(list.map((e) => e.name), ['keep.md']);
  assert.equal(formatRelativeTime(now - 10_000, now, 'ja'), 'たった今');
  assert.equal(formatRelativeTime(now - 3 * 60_000, now, 'ja'), '3分前');
  assert.equal(formatRelativeTime(now - 25 * 60 * 60_000, now, 'ja'), '昨日');
  assert.equal(formatRelativeTime(now - 3 * 60_000, now, 'en'), '3 minutes ago');
  assert.equal(formatRelativeTime(now - 25 * 60 * 60_000, now, 'en'), 'yesterday');
});

test('start-view rows use 再度開く when a local file has no handle', () => {
  const now = 10_000;
  const local = recordFromInput({ source: 'local', name: 'note.docx', openedAt: now - 60_000 });
  const row = toStartViewRecent(local, now, 'ja');
  assert.equal(row.badge, 'DOCX');
  assert.equal(row.action, 'open-recent:local:note.docx');
  assert.equal(row.subtitle, '再度開く · 1分前');
  assert.equal(canReopenDirectly(local), false);

  const handle = { name: 'note.docx' };
  const withHandle = recordFromInput({ source: 'local', name: 'note.docx', handle, openedAt: now - 60_000 });
  assert.equal(canReopenDirectly(withHandle), true);
  assert.equal(toStartViewRecent(withHandle, now, 'ja').subtitle, '1分前');

  const github = recordFromInput({
    source: 'github',
    name: 'spec.md',
    github: { owner: 'acme', repo: 'docs', ref: 'main', path: 'spec.md' },
    openedAt: now - 60_000,
  });
  assert.equal(toStartViewRecent(github, now, 'ja').subtitle, 'acme/docs · 1分前');
  assert.equal(canReopenDirectly(github), true);
});

test('resolveRecentOpen prefers a granted handle, then the picker', async () => {
  const handle = {
    name: 'note.md',
    queryPermission: async () => 'granted',
    getFile: async () => new File([new Uint8Array([65])], 'note.md'),
  };
  const withHandle = recordFromInput({ source: 'local', name: 'note.md', handle, openedAt: 1 });
  const granted = await resolveRecentOpen(withHandle, async () => { throw new Error('picker should not run'); });
  assert.equal(granted.mode, 'handle');
  assert.equal(granted.file.name, 'note.md');

  const denied = recordFromInput({
    source: 'local',
    name: 'note.md',
    handle: { name: 'note.md', queryPermission: async () => 'denied', requestPermission: async () => 'denied' },
    openedAt: 1,
  });
  const picked = await resolveRecentOpen(denied, async () => ({ name: 'other.md', bytes: new Uint8Array([1]) }));
  assert.equal(picked.mode, 'picker');
  assert.equal(picked.file.name, 'other.md');

  const github = recordFromInput({
    source: 'github',
    name: 'a.md',
    github: { owner: 'a', repo: 'b', ref: 'main', path: 'a.md' },
    openedAt: 1,
  });
  const viaGithub = await resolveRecentOpen(github);
  assert.equal(viaGithub.mode, 'github');
  assert.equal(viaGithub.github.path, 'a.md');

  assert.equal((await resolveRecentOpen(undefined)).mode, 'missing');
});
