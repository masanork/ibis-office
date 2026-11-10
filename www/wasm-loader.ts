// SPDX-License-Identifier: AGPL-3.0-only
// Reconstruct the wasm-bindgen module from content-addressed static assets.
interface Chunk { file: string; size: number; sha256: string }
interface Manifest { size: number; sha256: string; chunks: Chunk[] }

async function sha256(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', bytes as Uint8Array<ArrayBuffer>);
  return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('');
}

export async function loadWasm(manifestUrl: URL, progress: (loaded: number, total: number) => void): Promise<Response> {
  const response = await fetch(manifestUrl);
  if (!response.ok) throw new Error(`Wasm manifest: HTTP ${response.status}`);
  const manifest: Manifest = await response.json();
  if (!Number.isSafeInteger(manifest.size) || manifest.size <= 0 ||
      !/^[a-f0-9]{64}$/.test(manifest.sha256) || !Array.isArray(manifest.chunks) || !manifest.chunks.length ||
      manifest.chunks.some(c => !c || !Number.isSafeInteger(c.size) || c.size <= 0 || c.size > 20 * 1024 * 1024 ||
        !/^[a-f0-9]{64}$/.test(c.sha256) || c.file !== `ibis_office_app_bg.${c.sha256}.chunk`) ||
      manifest.chunks.reduce((sum, c) => sum + c.size, 0) !== manifest.size) {
    throw new Error('Invalid Wasm manifest');
  }
  let loaded = 0;
  const chunks = await Promise.all(manifest.chunks.map(async chunk => {
    const result = await fetch(new URL(chunk.file, manifestUrl));
    if (!result.ok) throw new Error(`${chunk.file}: HTTP ${result.status}`);
    const bytes = new Uint8Array(await result.arrayBuffer());
    if (bytes.byteLength !== chunk.size || await sha256(bytes) !== chunk.sha256) {
      throw new Error(`Invalid Wasm chunk: ${chunk.file}`);
    }
    loaded += bytes.byteLength;
    progress(loaded, manifest.size);
    return bytes;
  }));
  const combined = new Uint8Array(manifest.size);
  let offset = 0;
  for (const chunk of chunks) { combined.set(chunk, offset); offset += chunk.byteLength; }
  if (await sha256(combined) !== manifest.sha256) throw new Error('Invalid reconstructed Wasm');
  return new Response(combined, { headers: { 'Content-Type': 'application/wasm' } });
}
