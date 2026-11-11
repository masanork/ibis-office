// SPDX-License-Identifier: AGPL-3.0-only
import { readFileSync, existsSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';

const home = readFileSync(new URL('../../www/index.html', import.meta.url), 'utf8');
const redirect = home.match(/<script>([\s\S]*?)<\/script>/)?.[1];

test('existing editor links retain query and fragment', () => {
  let target;
  runInNewContext(redirect, { location: { search: '?open=%2Ftest.docx&view=1', hash: '#section', replace: value => { target = value; } } });
  assert.equal(target, './editor.html?open=%2Ftest.docx&view=1#section');
});

test('home stays put and links to the editor and a distributable sample', () => {
  let redirected = false;
  runInNewContext(redirect, { location: { search: '', hash: '', replace: () => { redirected = true; } } });
  assert.equal(redirected, false);
  assert.match(home, /href="\.\/editor\.html"/);
  assert.match(home, /href="\.\/editor\.html\?open=%2Fsamples%2Fsorane-welcome\.docx"/);
  assert.ok(existsSync(new URL('../../www/samples/sorane-welcome.docx', import.meta.url)));
});
