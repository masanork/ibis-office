// SPDX-License-Identifier: AGPL-3.0-only
// Export decrypted Spot records to TSV (for soraneDocs Raw sheet paste) and UTF-8 BOM CSV.

export interface SpotDecryptedRecord {
  envelope_id: string;
  source: string;
  payload: Record<string, unknown>;
}

function sanitizeTsvCell(value: string): string {
  return value.replace(/[\t\r\n]+/g, ' ');
}

function escapeCsvCell(value: string): string {
  if (/[",\r\n]/.test(value)) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

export function formatSpotTsv(records: SpotDecryptedRecord[]): string {
  return records
    .map((rec) => {
      const submittedAt = typeof rec.payload.submitted_at === 'string' ? rec.payload.submitted_at : '';
      const json = sanitizeTsvCell(JSON.stringify(rec.payload));
      return `${sanitizeTsvCell(rec.envelope_id)}\t${sanitizeTsvCell(submittedAt)}\t${json}`;
    })
    .join('\n');
}

export function formatSpotCsv(records: SpotDecryptedRecord[]): string {
  const answerKeys: string[] = [];
  const seen = new Set<string>();
  for (const rec of records) {
    const answers = rec.payload.answers;
    if (answers && typeof answers === 'object' && !Array.isArray(answers)) {
      for (const key of Object.keys(answers as Record<string, unknown>)) {
        if (!seen.has(key)) {
          seen.add(key);
          answerKeys.push(key);
        }
      }
    }
  }

  const header = ['record_id', 'submitted_at', 'source', 'trust_status', ...answerKeys];
  const lines = [header.map(escapeCsvCell).join(',')];
  for (const rec of records) {
    const submittedAt = typeof rec.payload.submitted_at === 'string' ? rec.payload.submitted_at : '';
    const verification =
      rec.payload.verification && typeof rec.payload.verification === 'object' && !Array.isArray(rec.payload.verification)
        ? (rec.payload.verification as Record<string, unknown>)
        : {};
    const trustStatus = typeof verification.trust_status === 'string' ? verification.trust_status : '';
    const answers =
      rec.payload.answers && typeof rec.payload.answers === 'object' && !Array.isArray(rec.payload.answers)
        ? (rec.payload.answers as Record<string, unknown>)
        : {};
    const row = [
      rec.envelope_id,
      submittedAt,
      rec.source,
      trustStatus,
      ...answerKeys.map((k) => {
        const val = answers[k];
        return val === null || val === undefined ? '' : String(val);
      }),
    ];
    lines.push(row.map(escapeCsvCell).join(','));
  }
  return `\uFEFF${lines.join('\r\n')}\r\n`;
}
