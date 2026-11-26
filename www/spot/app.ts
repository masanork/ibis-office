// SPDX-License-Identifier: AGPL-3.0-only
import {
  generateRecipientKeyPair,
  openSpotEnvelope,
  sealSpotEnvelope,
  type SpotRecipientJwk,
} from './cose-envelope.js';
import { formatSpotCsv, formatSpotTsv, type SpotDecryptedRecord } from './export.js';
import { renderQrSvg } from './qr.js';

interface FormField {
  id: string;
  label: string;
  type: 'text' | 'number' | 'textarea' | 'date';
  required?: boolean;
  placeholder?: string;
  claim_path?: string;
}

export interface SpotVerificationMeta {
  method: string;
  trust_status: 'verified' | 'unverified';
}

interface FormManifest {
  schema: string;
  tenant_id: string;
  form_id: string;
  version: string;
  title: string;
  description?: string;
  relay_url: string;
  inbox_url?: string;
  recipient_key?: SpotRecipientJwk;
  fields: FormField[];
}

interface QueuedEnvelope {
  envelope_id: string;
  tenant_id: string;
  form_id: string;
  envelope: string;
  created_at: number;
  status: 'queued' | 'sent';
}

const DB_NAME = 'sorane-spot-poc';
const STORE_QUEUE = 'envelopes';
const KEY_STORAGE = 'sorane.spot.poc.demo-keys.v1';

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE_QUEUE)) {
        db.createObjectStore(STORE_QUEUE, { keyPath: 'envelope_id' });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function putQueueItem(item: QueuedEnvelope): Promise<void> {
  const db = await openDb();
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(STORE_QUEUE, 'readwrite');
    tx.objectStore(STORE_QUEUE).put(item);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

async function listQueueItems(): Promise<QueuedEnvelope[]> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_QUEUE, 'readonly');
    const req = tx.objectStore(STORE_QUEUE).getAll();
    req.onsuccess = () => resolve((req.result as QueuedEnvelope[]).sort((a, b) => b.created_at - a.created_at));
    req.onerror = () => reject(req.error);
  });
}

async function ensureDemoKeys(kid: string): Promise<{ publicJwk: SpotRecipientJwk; privateJwk: SpotRecipientJwk }> {
  const saved = localStorage.getItem(KEY_STORAGE);
  if (saved) {
    try {
      return JSON.parse(saved) as { publicJwk: SpotRecipientJwk; privateJwk: SpotRecipientJwk };
    } catch {
      // regenerate below
    }
  }
  const pair = await generateRecipientKeyPair(kid);
  localStorage.setItem(KEY_STORAGE, JSON.stringify(pair));
  return pair;
}

async function loadManifest(): Promise<FormManifest> {
  const params = new URLSearchParams(location.search);
  const manifestUrl = params.get('manifest') || './sample-manifest.json';
  const res = await fetch(manifestUrl);
  if (!res.ok) throw new Error(`Failed to load manifest: ${res.status}`);
  return (await res.json()) as FormManifest;
}

async function main(): Promise<void> {
  if ('serviceWorker' in navigator) {
    void navigator.serviceWorker.register('./sw.js', { scope: './' }).catch(() => {});
  }

  const netBadge = document.getElementById('net-status')!;
  const updateOnline = () => {
    netBadge.textContent = navigator.onLine ? 'オンライン' : 'オフライン';
  };
  updateOnline();
  window.addEventListener('online', () => {
    updateOnline();
    void flushQueue();
  });
  window.addEventListener('offline', updateOnline);

  const manifest = await loadManifest();
  let demoKeys = await ensureDemoKeys(`${manifest.tenant_id}-2026-01`);
  const recipientPub = manifest.recipient_key ?? demoKeys.publicJwk;

  document.getElementById('form-title')!.textContent = manifest.title;
  document.getElementById('form-meta')!.textContent =
    `tenant: ${manifest.tenant_id} / form: ${manifest.form_id} v${manifest.version}`;
  const relayInput = document.getElementById('relay-override') as HTMLInputElement;
  relayInput.value = manifest.relay_url;

  const fieldsEl = document.getElementById('form-fields')!;
  for (const field of manifest.fields) {
    const label = document.createElement('label');
    label.htmlFor = `field-${field.id}`;
    label.textContent = field.required ? `${field.label} *` : field.label;
    fieldsEl.appendChild(label);

    const input = field.type === 'textarea' ? document.createElement('textarea') : document.createElement('input');
    input.id = `field-${field.id}`;
    input.name = field.id;
    if (input instanceof HTMLInputElement) {
      input.type = field.type === 'number' ? 'number' : field.type === 'date' ? 'date' : 'text';
    }
    if (field.required) input.required = true;
    if (field.placeholder) input.placeholder = field.placeholder;
    if (field.claim_path) input.dataset.claimPath = field.claim_path;
    fieldsEl.appendChild(input);
  }

  const renderQueue = async () => {
    const items = await listQueueItems();
    const listEl = document.getElementById('queue-list')!;
    if (items.length === 0) {
      listEl.textContent = 'キューは空です。';
      return;
    }
    listEl.innerHTML = '';
    for (const item of items) {
      const wrap = document.createElement('div');
      wrap.style.marginBottom = '12px';
      wrap.innerHTML = `<div><strong>${item.status === 'sent' ? '送信済み' : '未送信（待機中）'}</strong> — <code>${item.envelope_id.slice(0, 12)}...</code> (${item.envelope.length} chars)</div>`;
      const qrBox = document.createElement('div');
      qrBox.style.maxWidth = '220px';
      qrBox.style.margin = '8px 0';
      try {
        qrBox.innerHTML = renderQrSvg(item.envelope);
      } catch {
        qrBox.textContent = 'QR 上限超過（テキストまたは HTTP 提出を利用）';
      }
      wrap.appendChild(qrBox);
      const pre = document.createElement('pre');
      pre.textContent = item.envelope;
      wrap.appendChild(pre);
      listEl.appendChild(wrap);
    }
  };

  const flushQueue = async () => {
    const items = (await listQueueItems()).filter((i) => i.status === 'queued');
    if (items.length === 0) return;
    try {
      const res = await fetch(relayInput.value, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          tenant_id: manifest.tenant_id,
          items: items.map(({ envelope_id, form_id, envelope }) => ({ envelope_id, form_id, envelope })),
        }),
      });
      if (!res.ok) return;
      for (const item of items) {
        await putQueueItem({ ...item, status: 'sent' });
      }
      await renderQueue();
    } catch {
      // Offline or unreachable; keep queued in IndexedDB.
    }
  };

  document.getElementById('btn-gen-keys')!.addEventListener('click', async () => {
    demoKeys = await generateRecipientKeyPair(`${manifest.tenant_id}-2026-01`);
    localStorage.setItem(KEY_STORAGE, JSON.stringify(demoKeys));
    await renderQueue();
  });

  document.getElementById('spot-form')!.addEventListener('submit', async (event) => {
    event.preventDefault();
    const answers: Record<string, string> = {};
    for (const field of manifest.fields) {
      const el = document.getElementById(`field-${field.id}`) as HTMLInputElement | HTMLTextAreaElement;
      answers[field.id] = el.value;
    }
    const verification: SpotVerificationMeta = {
      method: 'self_asserted',
      trust_status: 'unverified',
    };
    const sealed = await sealSpotEnvelope(
      manifest.recipient_key ?? demoKeys.publicJwk,
      { tenantId: manifest.tenant_id, formId: manifest.form_id, version: manifest.version },
      { answers, verification, submitted_at: new Date().toISOString() },
    );
    await putQueueItem({
      envelope_id: sealed.envelopeId,
      tenant_id: manifest.tenant_id,
      form_id: manifest.form_id,
      envelope: sealed.envelope,
      created_at: Date.now(),
      status: 'queued',
    });
    await renderQueue();
    await flushQueue();
  });

  document.getElementById('btn-next-companion')!.addEventListener('click', () => {
    const carryIds = new Set(['address', 'shelter', 'input_helper', 'group_id']);
    const groupEl = document.getElementById('field-group_id') as HTMLInputElement | null;
    if (groupEl && !groupEl.value.trim()) {
      const bytes = crypto.getRandomValues(new Uint8Array(4));
      groupEl.value = `grp-${Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')}`;
    }
    for (const field of manifest.fields) {
      if (carryIds.has(field.id)) continue;
      const el = document.getElementById(`field-${field.id}`) as HTMLInputElement | HTMLTextAreaElement | null;
      if (el) el.value = '';
    }
    const first = document.getElementById('field-name') as HTMLInputElement | null;
    first?.focus();
  });

  document.getElementById('btn-flush')!.addEventListener('click', () => {
    void flushQueue();
  });

  const decryptedById = new Map<string, { payload: unknown; source: string }>();
  const renderDecrypted = () => {
    const target = document.getElementById('inbox-list')!;
    if (decryptedById.size === 0) {
      target.textContent = 'まだデータがありません。';
      return;
    }
    const table = document.createElement('table');
    table.innerHTML = '<thead><tr><th>ID</th><th>経路</th><th>復号ペイロード</th></tr></thead>';
    const tbody = document.createElement('tbody');
    for (const [id, entry] of decryptedById.entries()) {
      const tr = document.createElement('tr');
      tr.innerHTML = `<td><code>${id.slice(0, 12)}...</code></td><td>${entry.source}</td><td><pre>${JSON.stringify(entry.payload, null, 2)}</pre></td>`;
      tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    target.innerHTML = '';
    target.appendChild(table);
  };

  document.getElementById('btn-fetch-inbox')!.addEventListener('click', async () => {
    const url = `${manifest.inbox_url ?? '/api/relay/v1/inbox'}?tenant_id=${encodeURIComponent(manifest.tenant_id)}`;
    const res = await fetch(url);
    if (!res.ok) return;
    const data = (await res.json()) as { items: Array<{ envelope: string }> };
    for (const row of data.items) {
      try {
        const opened = await openSpotEnvelope(row.envelope, demoKeys.privateJwk, {
          tenantId: manifest.tenant_id,
          formId: manifest.form_id,
          version: manifest.version,
        });
        decryptedById.set(opened.envelopeId, { payload: opened.payload, source: 'relay' });
      } catch {
        // Ignore items encrypted for another key.
      }
    }
    renderDecrypted();
  });

  document.getElementById('btn-decrypt-paste')!.addEventListener('click', async () => {
    const input = (document.getElementById('paste-envelope') as HTMLTextAreaElement).value.trim();
    if (!input) return;
    const opened = await openSpotEnvelope(input, demoKeys.privateJwk, {
      tenantId: manifest.tenant_id,
      formId: manifest.form_id,
      version: manifest.version,
    });
    decryptedById.set(opened.envelopeId, { payload: opened.payload, source: 'offline-paste' });
    renderDecrypted();
  });

  let scanStream: MediaStream | null = null;
  let scanActive = false;
  const scannerBox = document.getElementById('qr-scanner')!;
  const scanVideo = document.getElementById('qr-video') as HTMLVideoElement;
  const scanStatus = document.getElementById('qr-scan-status')!;

  const stopScan = () => {
    scanActive = false;
    if (scanStream) {
      for (const track of scanStream.getTracks()) track.stop();
      scanStream = null;
    }
    scanVideo.srcObject = null;
    scannerBox.classList.add('hidden');
  };

  document.getElementById('btn-stop-scan')!.addEventListener('click', stopScan);

  document.getElementById('btn-scan-qr')!.addEventListener('click', async () => {
    const Detector = (window as unknown as {
      BarcodeDetector?: new (opts: { formats: string[] }) => {
        detect(source: HTMLVideoElement): Promise<Array<{ rawValue?: string }>>;
      };
    }).BarcodeDetector;
    if (!Detector || !navigator.mediaDevices?.getUserMedia) {
      scanStatus.textContent = 'このブラウザはカメラ QR 読取に未対応です（下の文字列貼付をご利用ください）。';
      scannerBox.classList.remove('hidden');
      return;
    }
    try {
      scanStream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' } });
      scanVideo.srcObject = scanStream;
      await scanVideo.play();
      scannerBox.classList.remove('hidden');
      scanStatus.textContent = 'QR コードをかざしてください...';
      scanActive = true;
      const detector = new Detector({ formats: ['qr_code'] });
      const tick = async () => {
        if (!scanActive) return;
        try {
          const codes = await detector.detect(scanVideo);
          for (const code of codes) {
            const raw = code.rawValue?.trim() ?? '';
            if (!raw.startsWith('spot1:')) continue;
            const opened = await openSpotEnvelope(raw, demoKeys.privateJwk, {
              tenantId: manifest.tenant_id,
              formId: manifest.form_id,
              version: manifest.version,
            });
            decryptedById.set(opened.envelopeId, { payload: opened.payload, source: 'qr-camera' });
            renderDecrypted();
            scanStatus.textContent = `読取成功: ${opened.envelopeId.slice(0, 12)}...`;
          }
        } catch {
          // Continue scanning next frame.
        }
        if (scanActive) requestAnimationFrame(() => { void tick(); });
      };
      requestAnimationFrame(() => { void tick(); });
    } catch {
      scanStatus.textContent = 'カメラを起動できませんでした。';
      scannerBox.classList.remove('hidden');
    }
  });

  const currentRecords = (): SpotDecryptedRecord[] =>
    Array.from(decryptedById.entries()).map(([envelope_id, entry]) => ({
      envelope_id,
      source: entry.source,
      payload:
        entry.payload && typeof entry.payload === 'object' && !Array.isArray(entry.payload)
          ? (entry.payload as Record<string, unknown>)
          : { value: entry.payload },
    }));

  document.getElementById('btn-copy-tsv')!.addEventListener('click', async () => {
    const records = currentRecords();
    if (records.length === 0) return;
    const tsv = formatSpotTsv(records);
    await navigator.clipboard.writeText(tsv);
  });

  document.getElementById('btn-download-csv')!.addEventListener('click', () => {
    const records = currentRecords();
    if (records.length === 0) return;
    const csv = formatSpotCsv(records);
    const blob = new Blob([csv], { type: 'text/csv;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${manifest.tenant_id}-${manifest.form_id}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  });

  const tabSubmit = document.getElementById('tab-submit')!;
  const tabReceive = document.getElementById('tab-receive')!;
  const panelSubmit = document.getElementById('panel-submit')!;
  const panelReceive = document.getElementById('panel-receive')!;
  tabSubmit.addEventListener('click', () => {
    tabSubmit.classList.add('active');
    tabReceive.classList.remove('active');
    panelSubmit.classList.remove('hidden');
    panelReceive.classList.add('hidden');
  });
  tabReceive.addEventListener('click', () => {
    tabReceive.classList.add('active');
    tabSubmit.classList.remove('active');
    panelReceive.classList.remove('hidden');
    panelSubmit.classList.add('hidden');
  });

  void recipientPub;
  await renderQueue();
}

void main();
