// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

import init, { decode_heic_png } from './pkg/heic/ibis_heic_web.js';

self.onmessage = async ({data}: MessageEvent<Uint8Array>) => {
  try {
    await init();
    const png = decode_heic_png(data);
    self.postMessage({png}, {transfer: [png.buffer]});
  } catch (error) {
    const reason = String(error) === 'HEIC image exceeds the preview dimensions'
      ? 'dimensions'
      : 'decode';
    self.postMessage({reason});
  }
};
