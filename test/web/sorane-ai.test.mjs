// SPDX-License-Identifier: AGPL-3.0-only
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';

test('Workers AI bridge forwards completion deltas without the final response summary', async () => {
  const delta = 'data: {"choices":[{"delta":{"content":"OK"}}]}';
  const usage = 'data: {"choices":[],"usage":{"total_tokens":2}}';
  const summary = 'data: {"response":"OK","usage":{"total_tokens":2}}';
  const error = 'data: {"error":{"message":"failed"}}';
  const end = 'data: [DONE]';
  const pieces = [delta + '\r\n' + usage + '\n' + summary.slice(0, 20),
    summary.slice(20) + '\n' + error + '\n' + end];
  const body = new ReadableStream({
    start(controller) {
      const encoder = new TextEncoder();
      for (const piece of pieces) controller.enqueue(encoder.encode(piece));
      controller.close();
    },
  });
  const window = {};
  const script = await readFile(new URL('../../www/sorane-ai.js', import.meta.url), 'utf8');
  runInNewContext(script, { window, AbortController, TextDecoder, fetch: async () => new Response(body) });
  const lines = [];
  await window.ibisOfficeAiStream('{}', 'csrf', line => lines.push(line));
  assert.deepEqual(lines, [delta, usage, error, end]);
});
