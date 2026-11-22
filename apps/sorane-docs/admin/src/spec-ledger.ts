// SPDX-License-Identifier: AGPL-3.0-only
import { parse } from 'csv-parse/browser/esm/sync';

export function parseTestLedger(source: string, path: string) {
  const rows: { line: number; label: string; ids: string[] }[] = [];
  let nextLine = 1;
  let fields: { width: number; requirement: number; test: number; title: number } | undefined;
  parse(source.replace(/\r\n?/g, '\n'), {
    bom: true,
    delimiter: /\.tsv$/i.test(path) ? '\t' : ',',
    record_delimiter: '\n',
    relax_column_count: true,
    on_record(record: string[], info) {
      const line = nextLine;
      nextLine = info.lines + 1;
      if (record.every(value => !value.trim())) return null;
      if (!fields) {
        const header = record.map(value => value.trim().toLowerCase());
        fields = {
          width: header.length,
          requirement: header.findIndex(value => ['requirement_id', '要求id', '要件id'].includes(value)),
          test: header.findIndex(value => ['test_id', '試験id', 'テストid'].includes(value)),
          title: header.findIndex(value => ['title', '試験名', 'テスト名'].includes(value)),
        };
        if (fields.requirement < 0 || fields.test < 0) throw new Error('Missing ledger columns');
        return null;
      }
      if (record.length !== fields.width) throw new Error('Inconsistent ledger columns');
      const testId = record[fields.test].trim();
      const ids = record[fields.requirement].split(/[\s,;、]+/).filter(Boolean);
      if (testId && ids.length && rows.length < 201) {
        const title = record[fields.title]?.trim().replace(/\s+/g, ' ');
        rows.push({ line, label: title ? testId + ' · ' + title : testId, ids });
      }
      return null;
    },
  });
  return rows;
}
