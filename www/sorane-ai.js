// SPDX-License-Identifier: AGPL-3.0-only
// Same-origin bridge for the browser assistant. The model call stays on the
// Worker. This page only carries the JSON body and the SSE lines.

let controller = null;

async function errorCode(response) {
  const text = await response.text();
  try {
    const body = JSON.parse(text);
    if (body && typeof body.error === 'string') return body.error;
  } catch { /* The body was not JSON. */ }
  return `HTTP ${response.status}`;
}

window.ibisOfficeAiModels = async () => {
  const response = await fetch('/api/ai/models', {
    credentials: 'same-origin',
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) throw new Error(await errorCode(response));
  return await response.text();
};

function forwardStreamLine(line, onLine) {
  if (line.startsWith('data:')) {
    try {
      const event = JSON.parse(line.slice(5));
      // Workers AI can append a separate response/usage summary after the
      // completion chunks. Its text duplicates deltas already shown.
      if (event && !('choices' in event) && 'response' in event &&
          event.usage && typeof event.usage === 'object' && !('error' in event)) return;
    } catch { /* Let the editor handle malformed chunks and the [DONE] marker. */ }
  }
  onLine(line);
}

window.ibisOfficeAiStream = async (body, csrf, onLine) => {
  controller?.abort();
  controller = new AbortController();
  let response;
  try {
    response = await fetch('/api/ai', {
      method: 'POST',
      credentials: 'same-origin',
      signal: controller.signal,
      headers: {
        'content-type': 'application/json',
        accept: 'text/event-stream',
        'x-csrf-token': csrf,
      },
      body,
    });
  } catch (error) {
    if (error && error.name === 'AbortError') throw new Error('aborted');
    throw error;
  }
  if (!response.ok || !response.body) throw new Error(await errorCode(response));
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let split = buffer.indexOf('\n');
      while (split >= 0) {
        const line = buffer.slice(0, split).replace(/\r$/, '');
        buffer = buffer.slice(split + 1);
        forwardStreamLine(line, onLine);
        split = buffer.indexOf('\n');
      }
    }
  } catch (error) {
    if (error && error.name === 'AbortError') throw new Error('aborted');
    throw error;
  }
  if (buffer) forwardStreamLine(buffer.replace(/\r$/, ''), onLine);
};

window.ibisOfficeAiAbort = () => {
  controller?.abort();
  controller = null;
};
