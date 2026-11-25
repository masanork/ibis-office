// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  acceptAttribute,
  downloadBytes,
  hasFileSystemAccess,
  mimeForFileName,
  saveLocalFile,
  setCurrentHandle,
  getCurrentHandle,
  FILE_HANDLER_GROUPS,
  OPEN_EXTENSIONS,
} from '../../www/local-file.js';

test('mime types and openable extensions cover Office and Markdown', () => {
  assert.equal(mimeForFileName('note.docx'), 'application/vnd.openxmlformats-officedocument.wordprocessingml.document');
  assert.equal(mimeForFileName('sheet.xlsx'), 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet');
  assert.equal(mimeForFileName('deck.pptx'), 'application/vnd.openxmlformats-officedocument.presentationml.presentation');
  assert.equal(mimeForFileName('memo.md'), 'text/markdown');
  assert.ok(OPEN_EXTENSIONS.includes('docx'));
  assert.ok(OPEN_EXTENSIONS.includes('md'));
  assert.ok(acceptAttribute().includes('.docx'));
  assert.ok(FILE_HANDLER_GROUPS.some((g) => g.description === 'Markdown'));
});

test('File System Access is detected only when both pickers exist', () => {
  assert.equal(hasFileSystemAccess({}), false);
  assert.equal(hasFileSystemAccess({ showOpenFilePicker: () => {}, showSaveFilePicker: () => {} }), true);
});

test('save in place uses the kept handle when permission is granted', async () => {
  const writes = [];
  const handle = {
    name: 'note.docx',
    queryPermission: async () => 'granted',
    createWritable: async () => ({
      write: async (bytes) => { writes.push(bytes); },
      close: async () => {},
      abort: async () => {},
    }),
  };
  setCurrentHandle(handle);
  const result = await saveLocalFile('note.docx', 'application/octet-stream', new Uint8Array([1, 2, 3]), false, {});
  assert.deepEqual(result, { name: 'note.docx', method: 'inplace' });
  assert.equal(writes.length, 1);
  assert.deepEqual([...writes[0]], [1, 2, 3]);
  setCurrentHandle(null);
  assert.equal(getCurrentHandle(), null);
});

test('save as uses the save picker when File System Access is available', async () => {
  const created = [];
  const api = {
    showSaveFilePicker: async ({ suggestedName }) => {
      const handle = {
        name: suggestedName,
        queryPermission: async () => 'granted',
        createWritable: async () => ({
          write: async (bytes) => { created.push({ name: suggestedName, bytes }); },
          close: async () => {},
          abort: async () => {},
        }),
      };
      return handle;
    },
  };
  setCurrentHandle(null);
  const result = await saveLocalFile('memo.md', 'text/markdown', new Uint8Array([9]), true, api);
  assert.equal(result.method, 'picker');
  assert.equal(result.name, 'memo.md');
  assert.equal(created.length, 1);
  assert.equal(getCurrentHandle()?.name, 'memo.md');
  setCurrentHandle(null);
});

test('Safari-style fallback downloads when no file picker exists', async () => {
  const clicks = [];
  const api = {
    document: {
      createElement: (tag) => {
        const el = { tag, style: {}, click: () => clicks.push(el), remove() {} };
        return el;
      },
      body: { appendChild() {} },
    },
  };
  setCurrentHandle(null);
  const result = await saveLocalFile('out.docx', 'application/octet-stream', new Uint8Array([4]), false, api);
  assert.equal(result.method, 'download');
  assert.equal(clicks.length, 1);
  assert.equal(clicks[0].download, 'out.docx');
});

test('a cancelled save picker returns null', async () => {
  const err = new Error('cancelled');
  err.name = 'AbortError';
  const api = { showSaveFilePicker: async () => { throw err; } };
  setCurrentHandle(null);
  assert.equal(await saveLocalFile('a.docx', 'application/octet-stream', new Uint8Array([1]), true, api), null);
});
