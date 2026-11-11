// SPDX-License-Identifier: AGPL-3.0-only
// Each subset response has the same family name. Give each response a distinct
// family so fontdb can retain and search all of the independent subsets.
export function namespaceSubset(input: Uint8Array, sequence: number): Uint8Array {
  if (sequence < 1 || sequence > 999999) throw new Error('Dynamic font limit reached');
  const data = new Uint8Array(input);
  const view = new DataView(data.buffer);
  if (data.length < 12 || ![0x00010000, 0x4f54544f].includes(view.getUint32(0))) throw new Error('Invalid subset font');
  const n = view.getUint16(4);
  if (12 + n * 16 > data.length) throw new Error('Invalid font directory');
  let nameRecord = -1, nameStart = 0, nameLength = 0, head = -1, headRecord = -1, headLength = 0;
  for (let i = 0; i < n; i++) {
    const at = 12 + i * 16;
    const tag = String.fromCharCode(...data.slice(at, at + 4));
    const start = view.getUint32(at + 8), length = view.getUint32(at + 12);
    if (start + length > data.length) throw new Error('Truncated font table');
    if (tag === 'name') { nameRecord = at; nameStart = start; nameLength = length; }
    if (tag === 'head') { head = start; headRecord = at; headLength = length; }
  }
  if (nameRecord < 0 || head < 0 || head + 12 > data.length) throw new Error('Missing font table');
  const id = String(sequence).padStart(6, '0');
  let families = 0;
  const replace = (from: string, to: string, wide: boolean) => {
    const encode = (s: string) => [...s].flatMap(c => wide ? [0, c.charCodeAt(0)] : [c.charCodeAt(0)]);
    const a = encode(from), b = encode(to);
    for (let i = nameStart; i <= nameStart + nameLength - a.length; i++) {
      if (a.every((v, j) => data[i + j] === v)) {
        data.set(b, i); i += a.length - 1;
        if (from.endsWith(' Subset')) families++;
      }
    }
  };
  for (const wide of [true, false]) {
    for (const prefix of ['IBS', 'GJS']) {
      replace(`${prefix} Subset`, `IBS ${id}`, wide);
      replace(`${prefix}Subset`, `IBS${id}`, wide);
    }
  }
  if (!families) throw new Error('Unexpected subset family');
  const checksum = (start: number, length: number): number => {
    let sum = 0;
    for (let i = 0; i < length; i += 4) {
      let value = 0;
      for (let j = 0; j < 4; j++) value = (value * 256 + (i + j < length ? data[start + i + j] : 0)) >>> 0;
      sum = (sum + value) >>> 0;
    }
    return sum;
  };
  view.setUint32(nameRecord + 4, checksum(nameStart, nameLength));
  view.setUint32(head + 8, 0);
  view.setUint32(headRecord + 4, checksum(head, headLength));
  view.setUint32(head + 8, (0xb1b0afba - checksum(0, data.length)) >>> 0);
  return data;
}

export class DynamicFonts {
  private readonly pending = new Set<string>();
  private readonly resolved = new Set<string>();
  private running = false;
  private sequence = 0;
  constructor(private readonly config: { endpoint: string; font: string }, private readonly install: (data: Uint8Array) => void, private readonly fetcher: typeof fetch = globalThis.fetch.bind(globalThis)) {}

  request(glyphs: string[]): void {
    for (const glyph of glyphs) if (glyph && !this.resolved.has(glyph)) this.pending.add(glyph);
    if (!this.running && this.pending.size) {
      this.running = true;
      setTimeout(() => { void this.flush(); }, 40);
    }
  }

  private async fetchGroup(glyphs: string[]): Promise<void> {
    const url = new URL(this.config.endpoint);
    url.searchParams.set('font', this.config.font);
    url.searchParams.set('format', 'otf');
    // GPUI's font loader rejects faces without a Latin 'm'. Include it in
    // each subset so the returned face can be loaded as a fallback.
    url.searchParams.set('text', glyphs.join('') + 'm');
    const response = await this.fetcher(url);
    if (response.status === 204 && glyphs.length > 1) {
      for (const glyph of glyphs) await this.fetchGroup([glyph]);
      return;
    }
    if (response.status === 204) { this.resolved.add(glyphs[0]); return; }
    if (!response.ok) throw new Error(`Dynamic font: HTTP ${response.status}`);
    const font = namespaceSubset(new Uint8Array(await response.arrayBuffer()), ++this.sequence);
    this.install(font);
    for (const glyph of glyphs) this.resolved.add(glyph);
  }

  private async flush(): Promise<void> {
    try {
      while (this.pending.size) {
        const batch: string[] = [];
        let count = 0;
        for (const glyph of this.pending) {
          const length = Array.from(glyph).length;
          if (batch.length && count + length > 127) break;
          this.pending.delete(glyph); batch.push(glyph); count += length;
        }
        const unresolved = batch.filter(glyph => !this.resolved.has(glyph));
        if (!unresolved.length) continue;
        for (let attempt = 0; attempt < 3; attempt++) {
          const remaining = unresolved.filter(glyph => !this.resolved.has(glyph));
          if (!remaining.length) break;
          try { await this.fetchGroup(remaining); break; }
          catch (error) {
            if (attempt === 2) console.error('Could not load a dynamic font', error);
            else await new Promise(resolve => setTimeout(resolve, 250 * 2 ** attempt));
          }
        }
      }
    } finally { this.running = false; }
  }
}

/** Fetch independent subsets for each requested face; never let an API
 * fallback silently replace an entire batch in the requested typeface. */
export class FaceSubsetProvider {
  private readonly pending = new Set<string>();
  private readonly resolved = new Set<string>();
  private readonly faceResolved = new Set<string>();
  private readonly adminCovered = new Set<string>();
  private sequence = 0;
  private running = false;
  constructor(private readonly endpoint: string,
              private readonly install: (data: Uint8Array, face: string) => void,
              private readonly fetcher: typeof fetch = globalThis.fetch.bind(globalThis)) {}

  request(glyphs: string[]): void {
    for (const glyph of glyphs) if (glyph && !this.resolved.has(glyph)) this.pending.add(glyph);
    if (!this.running && this.pending.size) {
      this.running = true;
      setTimeout(() => { void this.flush(); }, 40);
    }
  }

  private async fetchGroup(face: string, glyphs: string[]): Promise<void> {
    glyphs = glyphs.filter(glyph => !this.faceResolved.has(`${face}\0${glyph}`));
    if (!glyphs.length) return;
    const url = new URL(this.endpoint);
    url.searchParams.set('font', face);
    url.searchParams.set('format', 'otf');
    // The administrative font intentionally contains no Latin characters.
    url.searchParams.set('text', glyphs.join('') + (face === 'acgjm' ? '' : 'm'));
    const response = await this.fetcher(url);
    if (response.status === 429) {
      const seconds = Number(response.headers.get('Retry-After')) || 5;
      await new Promise(resolve => setTimeout(resolve, Math.min(Math.max(seconds, 1), 60) * 1000));
    }
    if (!response.ok && response.status !== 204) throw new Error(`Dynamic font: HTTP ${response.status}`);
    const actual = response.headers.get('X-Font-Id');
    if (response.status !== 204 && !actual) throw new Error('Dynamic font: missing source font header');
    if ((response.status === 204 || actual !== face) && glyphs.length > 1) {
      const mid = Math.floor(glyphs.length / 2);
      await this.fetchGroup(face, glyphs.slice(0, mid));
      await this.fetchGroup(face, glyphs.slice(mid));
      return;
    }
    if (response.status !== 204 && !(face === 'acgjm' && actual !== face)) {
      this.install(namespaceSubset(new Uint8Array(await response.arrayBuffer()), ++this.sequence), face);
      if (face === 'acgjm') for (const glyph of glyphs) this.adminCovered.add(glyph);
    }
    for (const glyph of glyphs) this.faceResolved.add(`${face}\0${glyph}`);
  }

  private async flush(): Promise<void> {
    try {
      while (this.pending.size) {
        const batch: string[] = [];
        let count = 0;
        for (const glyph of this.pending) {
          const length = Array.from(glyph).length;
          if (batch.length && count + length > 127) break;
          this.pending.delete(glyph); batch.push(glyph); count += length;
        }
        const work = batch.filter(glyph => !this.resolved.has(glyph)).sort();
        if (!work.length) continue;
        const administrative = work.filter(glyph => [...glyph].some(c => {
          const cp = c.codePointAt(0)!; return cp >= 0x100000 && cp <= 0x10fffd;
        }));
        let complete = false;
        for (let attempt = 0; attempt < 3; attempt++) {
          try {
            await this.fetchGroup('acgjm', administrative);
            const ordinary = work.filter(glyph => !this.adminCovered.has(glyph));
            // UI fallback also uses registered faces; register the UI's sans
            // face first while document runs keep their explicit face chains.
            await this.fetchGroup('NotoSansCJKJP', ordinary);
            await this.fetchGroup('ipamjm', ordinary);
            complete = true; break;
          } catch (error) {
            if (attempt === 2) console.error('Could not load a dynamic font', error);
            else await new Promise(resolve => setTimeout(resolve, 250 * 2 ** attempt));
          }
        }
        if (complete) for (const glyph of work) this.resolved.add(glyph);
      }
    } finally { this.running = false; }
  }
}
