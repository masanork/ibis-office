// SPDX-License-Identifier: AGPL-3.0-only
// Compress a Wasm asset without buffering the whole file in JavaScript.
import { createReadStream, createWriteStream, statSync } from 'node:fs';
import { pipeline } from 'node:stream/promises';
import { createBrotliCompress, constants } from 'node:zlib';
const [input, output] = process.argv.slice(2);
if (!input || !output) throw new Error('Usage: node compress-web-wasm.mjs INPUT OUTPUT');
await pipeline(createReadStream(input), createBrotliCompress({ params: {
  [constants.BROTLI_PARAM_QUALITY]: 5,
  [constants.BROTLI_PARAM_SIZE_HINT]: statSync(input).size,
} }), createWriteStream(output, { flags: 'wx' }));
