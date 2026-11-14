// SPDX-License-Identifier: AGPL-3.0-only
import { readFile } from 'node:fs/promises';

// Simulate a user who has already completed font setup for this build.
const versions = await readFile(new URL('../version.txt', import.meta.url), 'utf8');
export const wasmVersion = versions.match(/^wasm\s+(\S+)/m)?.[1];
if (!wasmVersion) throw new Error('version.txt does not contain a wasm version');
