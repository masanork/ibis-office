// SPDX-License-Identifier: AGPL-3.0-only
// Zero-dependency ISO/IEC 18004 QR Code encoder (Byte mode, ECC Level L, Versions 1..20) -> SVG.

// [totalCodewords, ecPerBlock, g1Blocks, g1Data, g2Blocks, g2Data] for Level L, Versions 1..20.
const VERSION_L_TABLE: ReadonlyArray<[number, number, number, number, number, number]> = [
  [26, 7, 1, 19, 0, 0],       // V1
  [44, 10, 1, 34, 0, 0],      // V2
  [70, 15, 1, 55, 0, 0],      // V3
  [100, 20, 1, 80, 0, 0],     // V4
  [134, 26, 1, 108, 0, 0],    // V5
  [172, 18, 2, 68, 0, 0],     // V6
  [196, 20, 2, 78, 0, 0],     // V7
  [242, 24, 2, 97, 0, 0],     // V8
  [292, 30, 2, 116, 0, 0],    // V9
  [346, 18, 2, 68, 2, 69],    // V10
  [404, 20, 4, 81, 0, 0],     // V11
  [466, 24, 2, 92, 2, 93],    // V12
  [532, 26, 4, 107, 0, 0],    // V13
  [581, 30, 3, 115, 1, 116],  // V14
  [655, 22, 5, 87, 1, 88],    // V15
  [733, 24, 5, 98, 1, 99],    // V16
  [815, 28, 1, 107, 5, 108],  // V17
  [901, 30, 5, 120, 1, 121],  // V18
  [991, 28, 3, 113, 4, 114],  // V19
  [1085, 28, 3, 107, 5, 108], // V20 (up to 858 bytes)
];

const ALIGNMENT_CENTERS: ReadonlyArray<number[]> = [
  [],
  [6, 18],
  [6, 22],
  [6, 26],
  [6, 30],
  [6, 34],
  [6, 22, 38],
  [6, 24, 42],
  [6, 26, 46],
  [6, 28, 50],
  [6, 30, 54],
  [6, 32, 58],
  [6, 34, 62],
  [6, 26, 46, 66],
  [6, 26, 48, 70],
  [6, 26, 50, 74],
  [6, 30, 54, 78],
  [6, 30, 56, 82],
  [6, 30, 58, 86],
  [6, 34, 62, 90],
];

const GF_EXP = new Uint8Array(512);
const GF_LOG = new Uint8Array(256);
(() => {
  let x = 1;
  for (let i = 0; i < 255; i++) {
    GF_EXP[i] = x;
    GF_LOG[x] = i;
    x <<= 1;
    if (x & 0x100) x ^= 0x11d;
  }
  for (let i = 255; i < 512; i++) {
    GF_EXP[i] = GF_EXP[i - 255];
  }
})();

function gfMul(a: number, b: number): number {
  if (a === 0 || b === 0) return 0;
  return GF_EXP[GF_LOG[a] + GF_LOG[b]];
}

function rsGeneratorPoly(degree: number): Uint8Array {
  let poly = new Uint8Array([1]);
  for (let i = 0; i < degree; i++) {
    const next = new Uint8Array(poly.length + 1);
    const factor = GF_EXP[i];
    for (let j = 0; j < poly.length; j++) {
      next[j] ^= poly[j];
      next[j + 1] ^= gfMul(poly[j], factor);
    }
    poly = next;
  }
  return poly;
}

function rsEncode(data: Uint8Array, ecCount: number): Uint8Array {
  const gen = rsGeneratorPoly(ecCount);
  const work = new Uint8Array(data.length + ecCount);
  work.set(data);
  for (let i = 0; i < data.length; i++) {
    const coef = work[i];
    if (coef === 0) continue;
    for (let j = 0; j < gen.length; j++) {
      work[i + j] ^= gfMul(gen[j], coef);
    }
  }
  return work.slice(data.length);
}

function bchFormatInfo(mask: number): number {
  const data = (0b01 << 3) | mask; // ECC L = 01
  let d = data << 10;
  const g = 0x537;
  for (let i = 14; i >= 10; i--) {
    if ((d >> i) & 1) d ^= g << (i - 10);
  }
  return ((data << 10) | d) ^ 0x5412;
}

function bchVersionInfo(version: number): number {
  let d = version << 12;
  const g = 0x1f25;
  for (let i = 17; i >= 12; i--) {
    if ((d >> i) & 1) d ^= g << (i - 12);
  }
  return (version << 12) | d;
}

function selectVersion(byteLen: number): number {
  for (let v = 1; v <= VERSION_L_TABLE.length; v++) {
    const [, , g1b, g1d, g2b, g2d] = VERSION_L_TABLE[v - 1];
    const dataCodewords = g1b * g1d + g2b * g2d;
    const countBits = v <= 9 ? 8 : 16;
    const capacityBytes = Math.floor((dataCodewords * 8 - 4 - countBits) / 8);
    if (byteLen <= capacityBytes) return v;
  }
  throw new Error(`Payload too large for QR v20-L (${byteLen} bytes)`);
}

function buildCodewords(bytes: Uint8Array, version: number): Uint8Array {
  const [totalCw, ecPerBlock, g1b, g1d, g2b, g2d] = VERSION_L_TABLE[version - 1];
  const dataCwCount = g1b * g1d + g2b * g2d;
  const bits: number[] = [];
  const pushBits = (val: number, len: number) => {
    for (let i = len - 1; i >= 0; i--) bits.push((val >> i) & 1);
  };

  pushBits(0b0100, 4); // Byte mode
  pushBits(bytes.length, version <= 9 ? 8 : 16);
  for (const b of bytes) pushBits(b, 8);

  const maxBits = dataCwCount * 8;
  const termLen = Math.min(4, maxBits - bits.length);
  for (let i = 0; i < termLen; i++) bits.push(0);
  while (bits.length % 8 !== 0) bits.push(0);

  const dataCw = new Uint8Array(dataCwCount);
  for (let i = 0; i < bits.length; i += 8) {
    let val = 0;
    for (let j = 0; j < 8; j++) val = (val << 1) | bits[i + j];
    dataCw[i / 8] = val;
  }
  let padIdx = bits.length / 8;
  const pads = [0xec, 0x11];
  let p = 0;
  while (padIdx < dataCwCount) {
    dataCw[padIdx++] = pads[p++ & 1];
  }

  const dataBlocks: Uint8Array[] = [];
  const ecBlocks: Uint8Array[] = [];
  let offset = 0;
  for (let i = 0; i < g1b; i++) {
    const blk = dataCw.slice(offset, offset + g1d);
    offset += g1d;
    dataBlocks.push(blk);
    ecBlocks.push(rsEncode(blk, ecPerBlock));
  }
  for (let i = 0; i < g2b; i++) {
    const blk = dataCw.slice(offset, offset + g2d);
    offset += g2d;
    dataBlocks.push(blk);
    ecBlocks.push(rsEncode(blk, ecPerBlock));
  }

  const out = new Uint8Array(totalCw);
  let outIdx = 0;
  const maxDataLen = Math.max(g1d, g2d);
  for (let i = 0; i < maxDataLen; i++) {
    for (const blk of dataBlocks) {
      if (i < blk.length) out[outIdx++] = blk[i];
    }
  }
  for (let i = 0; i < ecPerBlock; i++) {
    for (const blk of ecBlocks) out[outIdx++] = blk[i];
  }
  return out;
}

export interface QrMatrix {
  version: number;
  size: number;
  modules: Uint8Array; // 1 = dark, 0 = light (row-major)
}

function buildMatrix(version: number, codewords: Uint8Array): QrMatrix {
  const size = version * 4 + 17;
  const grid = new Int8Array(size * size).fill(-1);
  const reserved = new Uint8Array(size * size);

  const setFixed = (r: number, c: number, dark: boolean) => {
    if (r < 0 || c < 0 || r >= size || c >= size) return;
    grid[r * size + c] = dark ? 1 : 0;
    reserved[r * size + c] = 1;
  };

  const placeFinder = (top: number, left: number) => {
    for (let r = -1; r <= 7; r++) {
      for (let c = -1; c <= 7; c++) {
        const dark =
          r >= 0 && r <= 6 && c >= 0 && c <= 6 &&
          (r === 0 || r === 6 || c === 0 || c === 6 || (r >= 2 && r <= 4 && c >= 2 && c <= 4));
        setFixed(top + r, left + c, dark);
      }
    }
  };
  placeFinder(0, 0);
  placeFinder(0, size - 7);
  placeFinder(size - 7, 0);

  for (let i = 8; i < size - 8; i++) {
    setFixed(6, i, i % 2 === 0);
    setFixed(i, 6, i % 2 === 0);
  }

  const centers = ALIGNMENT_CENTERS[version - 1];
  for (const cr of centers) {
    for (const cc of centers) {
      if (reserved[cr * size + cc]) continue;
      for (let r = -2; r <= 2; r++) {
        for (let c = -2; c <= 2; c++) {
          const dark = Math.abs(r) === 2 || Math.abs(c) === 2 || (r === 0 && c === 0);
          setFixed(cr + r, cc + c, dark);
        }
      }
    }
  }

  setFixed(size - 8, 8, true); // dark module

  // Reserve format info areas
  for (let i = 0; i < 9; i++) {
    if (!reserved[8 * size + i]) setFixed(8, i, false);
    if (!reserved[i * size + 8]) setFixed(i, 8, false);
  }
  for (let i = 0; i < 8; i++) {
    setFixed(8, size - 1 - i, false);
    setFixed(size - 1 - i, 8, false);
  }

  if (version >= 7) {
    const vInfo = bchVersionInfo(version);
    for (let i = 0; i < 18; i++) {
      const dark = ((vInfo >> i) & 1) === 1;
      const r = Math.floor(i / 3);
      const c = (i % 3) + size - 11;
      setFixed(r, c, dark);
      setFixed(c, r, dark);
    }
  }

  // Place data bits
  let bitIdx = 0;
  const totalBits = codewords.length * 8;
  let upward = true;
  for (let col = size - 1; col >= 1; col -= 2) {
    if (col === 6) col--;
    for (let step = 0; step < size; step++) {
      const r = upward ? size - 1 - step : step;
      for (const dc of [0, -1]) {
        const c = col + dc;
        if (reserved[r * size + c]) continue;
        let bit = 0;
        if (bitIdx < totalBits) {
          bit = (codewords[bitIdx >> 3] >> (7 - (bitIdx & 7))) & 1;
          bitIdx++;
        }
        grid[r * size + c] = bit;
      }
    }
    upward = !upward;
  }

  const maskFn = (mask: number, r: number, c: number): boolean => {
    switch (mask) {
      case 0: return (r + c) % 2 === 0;
      case 1: return r % 2 === 0;
      case 2: return c % 3 === 0;
      case 3: return (r + c) % 3 === 0;
      case 4: return (Math.floor(r / 2) + Math.floor(c / 3)) % 2 === 0;
      case 5: return ((r * c) % 2) + ((r * c) % 3) === 0;
      case 6: return (((r * c) % 2) + ((r * c) % 3)) % 2 === 0;
      default: return (((r + c) % 2) + ((r * c) % 3)) % 2 === 0;
    }
  };

  let bestMask = 0;
  let bestScore = Infinity;
  let bestModules = new Uint8Array(size * size);

  for (let mask = 0; mask < 8; mask++) {
    const candidate = new Uint8Array(size * size);
    for (let r = 0; r < size; r++) {
      for (let c = 0; c < size; c++) {
        const idx = r * size + c;
        const raw = grid[idx] === 1;
        candidate[idx] = !reserved[idx] && maskFn(mask, r, c) ? (raw ? 0 : 1) : (raw ? 1 : 0);
      }
    }
    const fmt = bchFormatInfo(mask);
    for (let i = 0; i < 15; i++) {
      const dark = ((fmt >> i) & 1) === 1 ? 1 : 0;
      if (i < 6) candidate[8 * size + i] = dark;
      else if (i < 8) candidate[8 * size + i + 1] = dark;
      else candidate[8 * size + (size - 15 + i)] = dark;

      if (i < 8) candidate[(size - 1 - i) * size + 8] = dark;
      else if (i < 9) candidate[7 * size + 8] = dark;
      else candidate[(14 - i) * size + 8] = dark;
    }

    // Simple penalty: runs of >= 5 + balance
    let penalty = 0;
    let darkCount = 0;
    for (let r = 0; r < size; r++) {
      let run = 1;
      for (let c = 0; c < size; c++) {
        const val = candidate[r * size + c];
        if (val) darkCount++;
        if (c > 0 && val === candidate[r * size + c - 1]) {
          run++;
          if (run === 5) penalty += 3;
          else if (run > 5) penalty += 1;
        } else {
          run = 1;
        }
      }
    }
    for (let c = 0; c < size; c++) {
      let run = 1;
      for (let r = 1; r < size; r++) {
        if (candidate[r * size + c] === candidate[(r - 1) * size + c]) {
          run++;
          if (run === 5) penalty += 3;
          else if (run > 5) penalty += 1;
        } else {
          run = 1;
        }
      }
    }
    const ratioDiff = Math.abs((darkCount * 100) / (size * size) - 50);
    penalty += Math.floor(ratioDiff / 5) * 10;

    if (penalty < bestScore) {
      bestScore = penalty;
      bestMask = mask;
      bestModules = candidate;
    }
  }
  void bestMask;
  return { version, size, modules: bestModules };
}

export function encodeQrMatrix(text: string): QrMatrix {
  const bytes = new TextEncoder().encode(text);
  const version = selectVersion(bytes.length);
  const codewords = buildCodewords(bytes, version);
  return buildMatrix(version, codewords);
}

export function renderQrSvg(text: string, quietZone = 4): string {
  const { size, modules } = encodeQrMatrix(text);
  const total = size + quietZone * 2;
  const rects: string[] = [];
  for (let r = 0; r < size; r++) {
    for (let c = 0; c < size; c++) {
      if (modules[r * size + c]) {
        rects.push(`M${c + quietZone},${r + quietZone}h1v1h-1z`);
      }
    }
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${total} ${total}" shape-rendering="crispEdges" role="img" aria-label="QR Code"><rect width="${total}" height="${total}" fill="#fff"/><path d="${rects.join('')}" fill="#000"/></svg>`;
}
