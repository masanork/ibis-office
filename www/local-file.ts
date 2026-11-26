// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
// File System Access API helpers: open/save local files, with a download fallback.

export type PickedLocalFile = {
  name: string;
  bytes: Uint8Array;
  handle?: FileSystemFileHandle;
};

export type SaveLocalResult = {
  name: string;
  method: 'inplace' | 'picker' | 'download';
};

export const OPEN_EXTENSIONS = [
  'docx', 'docm', 'dotx', 'dotm', 'doc', 'dot',
  'xlsx', 'xlsm', 'xlsb', 'xls', 'xltx', 'xltm', 'xlk', 'csv', 'tsv', 'tab',
  'pptx', 'pptm', 'ppsx', 'ppsm', 'potx', 'potm', 'ppt', 'pps', 'pot',
  'odp', 'otp', 'ods', 'ots',
  'md', 'markdown',
  'vsdx', 'epub', 'pdf', 'jtd', 'jtt', 'jttc',
  'hwp', 'hwpx', 'uot', 'wpd', 'wp6',
  'json', 'xml', 'sd-jwt', 'mdoc', 'ofd', 'xps', 'oxps', 'xdw', 'xbd',
] as const;

export const FILE_HANDLER_GROUPS: { description: string; accept: Record<string, string[]> }[] = [
  {
    description: 'Word',
    accept: {
      'application/vnd.openxmlformats-officedocument.wordprocessingml.document': ['.docx', '.docm', '.dotx', '.dotm'],
      'application/msword': ['.doc', '.dot'],
    },
  },
  {
    description: 'Excel',
    accept: {
      'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet': ['.xlsx', '.xlsm', '.xltx', '.xltm'],
      'application/vnd.ms-excel': ['.xls', '.xlk'],
      'text/csv': ['.csv'],
      'text/tab-separated-values': ['.tsv', '.tab'],
    },
  },
  {
    description: 'PowerPoint',
    accept: {
      'application/vnd.openxmlformats-officedocument.presentationml.presentation': ['.pptx', '.pptm', '.ppsx', '.ppsm', '.potx', '.potm'],
      'application/vnd.ms-powerpoint': ['.ppt', '.pps', '.pot'],
    },
  },
  {
    description: 'Markdown',
    accept: { 'text/markdown': ['.md', '.markdown'] },
  },
  {
    description: 'Lawtext',
    accept: {
      'text/plain': ['.lawtext', '.txt'],
      'application/xml': ['.xml'],
    },
  },
  {
    description: 'OpenDocument',
    accept: {
      'application/vnd.oasis.opendocument.spreadsheet': ['.ods', '.ots'],
      'application/vnd.oasis.opendocument.presentation': ['.odp', '.otp'],
    },
  },
  {
    description: 'DocuWorks',
    accept: {
      'application/vnd.fujifilm.docuworks': ['.xdw'],
      'application/vnd.fujifilm.docuworks.binder': ['.xbd'],
    },
  },
  {
    description: 'Other documents',
    accept: {
      'application/vnd.visio': ['.vsdx'],
      'application/epub+zip': ['.epub'],
      'application/pdf': ['.pdf'],
    },
  },
];

const MIME_BY_EXT: Record<string, string> = {
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  docm: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  dotx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.template',
  dotm: 'application/vnd.openxmlformats-officedocument.wordprocessingml.template',
  doc: 'application/msword',
  dot: 'application/msword',
  xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  xlsm: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  xltx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.template',
  xltm: 'application/vnd.openxmlformats-officedocument.spreadsheetml.template',
  xls: 'application/vnd.ms-excel',
  csv: 'text/csv',
  tsv: 'text/tab-separated-values',
  tab: 'text/tab-separated-values',
  pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  pptm: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  ppsx: 'application/vnd.openxmlformats-officedocument.presentationml.slideshow',
  md: 'text/markdown',
  markdown: 'text/markdown',
  lawtext: 'text/plain; charset=utf-8',
  txt: 'text/plain; charset=utf-8',
  xml: 'application/xml; charset=utf-8',
  vsdx: 'application/vnd.ms-visio.drawing.main+xml',
  pdf: 'application/pdf',
  epub: 'application/epub+zip',
  ods: 'application/vnd.oasis.opendocument.spreadsheet',
  odp: 'application/vnd.oasis.opendocument.presentation',
};

export function mimeForFileName(name: string): string {
  const ext = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1).toLowerCase() : '';
  return MIME_BY_EXT[ext] ?? 'application/octet-stream';
}

export function hasFileSystemAccess(api: {
  showOpenFilePicker?: unknown;
  showSaveFilePicker?: unknown;
} = globalThis as unknown as { showOpenFilePicker?: unknown; showSaveFilePicker?: unknown }): boolean {
  return typeof api.showOpenFilePicker === 'function' && typeof api.showSaveFilePicker === 'function';
}

let currentHandle: FileSystemFileHandle | null = null;

export function getCurrentHandle(): FileSystemFileHandle | null {
  return currentHandle;
}

export function setCurrentHandle(handle: FileSystemFileHandle | null): void {
  currentHandle = handle;
}

export function acceptAttribute(): string {
  return OPEN_EXTENSIONS.map((ext) => `.${ext}`).join(',');
}

export async function fileFromHandle(handle: FileSystemFileHandle): Promise<PickedLocalFile> {
  const file = await handle.getFile();
  return { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()), handle };
}

type PickerWindow = Window & {
  showOpenFilePicker?: (options?: unknown) => Promise<FileSystemFileHandle[]>;
  showSaveFilePicker?: (options?: unknown) => Promise<FileSystemFileHandle>;
};

function isAbort(error: unknown): boolean {
  return typeof error === 'object' && error !== null && (error as { name?: string }).name === 'AbortError';
}

export async function pickLocalFile(
  options: { remember?: boolean; accept?: string } = {},
  api: PickerWindow = window,
): Promise<PickedLocalFile | null> {
  const remember = options.remember !== false;
  if (!options.accept && typeof api.showOpenFilePicker === 'function') {
    try {
      const [handle] = await api.showOpenFilePicker({
        id: 'sorane-docs-open',
        multiple: false,
        types: FILE_HANDLER_GROUPS,
        excludeAcceptAllOption: false,
      });
      const picked = await fileFromHandle(handle);
      if (remember) setCurrentHandle(handle);
      return picked;
    } catch (error) {
      if (isAbort(error)) return null;
      // Fall through to the input picker when the API is present but refused.
    }
  }
  return pickWithInput(options.accept ?? acceptAttribute(), remember, api.document);
}

function pickWithInput(accept: string, remember: boolean, doc: Document | undefined): Promise<PickedLocalFile | null> {
  const documentRef = doc ?? document;
  return new Promise((resolve, reject) => {
    const input = documentRef.createElement('input');
    input.type = 'file';
    if (accept) input.accept = accept;
    input.addEventListener('change', () => {
      const file = input.files?.[0];
      if (!file) {
        resolve(null);
        return;
      }
      if (remember) setCurrentHandle(null);
      file.arrayBuffer().then((buffer) => resolve({ name: file.name, bytes: new Uint8Array(buffer) }), reject);
    });
    input.addEventListener('cancel', () => resolve(null));
    input.click();
  });
}

export async function queryWritePermission(handle: FileSystemFileHandle): Promise<boolean> {
  const withPerm = handle as FileSystemFileHandle & {
    queryPermission?: (descriptor: { mode: string }) => Promise<PermissionState>;
    requestPermission?: (descriptor: { mode: string }) => Promise<PermissionState>;
  };
  if (typeof withPerm.queryPermission !== 'function') return true;
  const current = await withPerm.queryPermission({ mode: 'readwrite' });
  if (current === 'granted') return true;
  if (typeof withPerm.requestPermission !== 'function') return false;
  return (await withPerm.requestPermission({ mode: 'readwrite' })) === 'granted';
}

export async function writeHandle(handle: FileSystemFileHandle, bytes: Uint8Array): Promise<void> {
  if (!(await queryWritePermission(handle))) {
    throw new Error('Write permission was not granted');
  }
  const writable = await handle.createWritable();
  try {
    await writable.write(bytes as BufferSource);
    await writable.close();
  } catch (error) {
    await writable.abort().catch(() => undefined);
    throw error;
  }
}

export function downloadBytes(name: string, mime: string, bytes: Uint8Array, doc: Document = document): void {
  const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mime }));
  const link = doc.createElement('a');
  link.href = url;
  link.download = name;
  link.style.display = 'none';
  doc.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 30000);
}

export async function saveLocalFile(
  name: string,
  mime: string,
  bytes: Uint8Array,
  saveAs: boolean,
  api: PickerWindow = window,
): Promise<SaveLocalResult | null> {
  const copy = bytes.slice();
  if (!saveAs && currentHandle) {
    try {
      await writeHandle(currentHandle, copy);
      return { name: currentHandle.name || name, method: 'inplace' };
    } catch (error) {
      if (isAbort(error)) return null;
      // Permission or a missing file: try a picker or a download next.
    }
  }
  if (typeof api.showSaveFilePicker === 'function') {
    try {
      const ext = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1).toLowerCase() : '';
      const handle = await api.showSaveFilePicker({
        id: 'sorane-docs-save',
        suggestedName: name,
        types: ext
          ? [{ description: name, accept: { [mime || mimeForFileName(name)]: [`.${ext}`] } }]
          : FILE_HANDLER_GROUPS,
      });
      await writeHandle(handle, copy);
      setCurrentHandle(handle);
      return { name: handle.name || name, method: 'picker' };
    } catch (error) {
      if (isAbort(error)) return null;
    }
  }
  downloadBytes(name, mime || mimeForFileName(name), copy, api.document);
  return { name, method: 'download' };
}

export async function handleFromDropItem(item: { getAsFileSystemHandle?: () => Promise<FileSystemHandle | null> } | DataTransferItem): Promise<FileSystemFileHandle | null> {
  const candidate = item as { getAsFileSystemHandle?: () => Promise<FileSystemHandle | null> };
  if (typeof candidate.getAsFileSystemHandle !== 'function') return null;
  const handle = await candidate.getAsFileSystemHandle();
  if (handle && handle.kind === 'file') return handle as FileSystemFileHandle;
  return null;
}

type LaunchParams = { files?: FileSystemFileHandle[] };
type LaunchQueue = { setConsumer: (consumer: (params: LaunchParams) => void) => void };

export function installLaunchQueue(
  onFile: (file: PickedLocalFile) => void,
  api: { launchQueue?: LaunchQueue } = window as unknown as { launchQueue?: LaunchQueue },
): Promise<PickedLocalFile | null> {
  return new Promise((resolve) => {
    const queue = api.launchQueue;
    if (!queue || typeof queue.setConsumer !== 'function') {
      resolve(null);
      return;
    }
    let resolved = false;
    const finish = (file: PickedLocalFile | null) => {
      if (resolved) {
        if (file) onFile(file);
        return;
      }
      resolved = true;
      resolve(file);
    };
    queue.setConsumer((params) => {
      void (async () => {
        const handle = params.files?.[0];
        if (!handle) {
          finish(null);
          return;
        }
        try {
          const picked = await fileFromHandle(handle);
          setCurrentHandle(handle);
          const already = resolved;
          finish(picked);
          if (already) onFile(picked);
        } catch {
          finish(null);
        }
      })();
    });
    // Chrome usually delivers the launch file immediately. After a short wait,
    // startup continues with no file (the start view).
    setTimeout(() => finish(null), 200);
  });
}
