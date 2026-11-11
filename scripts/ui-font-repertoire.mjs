// SPDX-License-Identifier: AGPL-3.0-only
// Read display values, never resource keys, comments, source code or documents.
import { readFile, readdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
export const uiSources = [
  resolve(root, 'crates/ibis-office-app/i18n'),
  resolve(root, 'forks/gpui-component/locales/ui.yml'),
];

export async function uiRepertoire(sources = uiSources) {
  const values = [];
  async function read(path) {
    if (!/\.(tsv|yml)$/.test(path)) {
      for (const entry of (await readdir(path, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
        if (entry.isDirectory() || entry.name.endsWith('.tsv')) await read(join(path, entry.name));
      }
      return;
    }
    for (const line of (await readFile(path, 'utf8')).split(/\r?\n/)) {
      if (path.endsWith('.tsv')) {
        if (!line || line.startsWith('#') || !line.includes('\t')) continue;
        values.push(line.slice(line.indexOf('\t') + 1));
      } else {
        // gpui-component's v2 locale resource uses one scalar per language.
        const match = line.match(/^\s+(?:en|ja):\s*(.+)$/);
        if (match) {
          let value = match[1];
          if (value.startsWith('"')) value = JSON.parse(value);
          else if (value.startsWith("'")) value = value.slice(1, -1).replaceAll("''", "'");
          if (['|', '>', '|-', '>-'].includes(value)) throw new Error('Unsupported multiline UI resource: ' + path);
          values.push(value);
        }
      }
    }
  }
  for (const source of sources) await read(source);
  const segmenter = new Intl.Segmenter('en', { granularity: 'grapheme' });
  return [...new Set(values.flatMap(value => [...segmenter.segment(value)]
    .map(({ segment }) => segment).filter(text => !/^[\p{Cc}\p{Cf}]+$/u.test(text))))];
}
