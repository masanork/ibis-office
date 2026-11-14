// SPDX-License-Identifier: AGPL-3.0-only
// Node build/test loader. Workers import the same generated module directly.
import { readFile } from 'node:fs/promises';
let ready;
export function loadSubsetter() {
  if (!ready) {
    ready = (async () => {
      const bg = await import('../crates/ibis-font-subsetter/pkg/ibis_font_subsetter_bg.js');
      const bytes = await readFile(new URL('../crates/ibis-font-subsetter/pkg/ibis_font_subsetter_bg.wasm', import.meta.url));
      const { instance } = await WebAssembly.instantiate(bytes, { './ibis_font_subsetter_bg.js': bg });
      bg.__wbg_set_wasm(instance.exports);
      instance.exports.__wbindgen_start?.();
      return bg;
    })().catch(error => { ready = undefined; throw error; });
  }
  return ready;
}
