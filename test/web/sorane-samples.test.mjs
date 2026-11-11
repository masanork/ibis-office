// SPDX-License-Identifier: AGPL-3.0-only
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
const source = readFileSync(new URL('../../www/sorane-samples.js',import.meta.url),'utf8');
function node() { return {children:[], append(...children) { this.children.push(...children); }}; }
async function render(payload, ok = true) {
  const container=node();
  const document={querySelector:()=>container,createElement:()=>node()};
  await runInNewContext(`(async () => { ${source} })()`,{document,fetch:async()=>({ok,json:async()=>payload}),encodeURIComponent});
  return container.children;
}
test('unavailable sample service leaves the bundled sample alone',async()=>{assert.equal((await render([],false)).length,0)});
test('only valid sample paths are built and titles are text, not HTML',async()=>{
  const records=await render([{id:'11111111-1111-4111-8111-111111111111',name:'sample.docx',title:'<img onerror=alert(1)>'},{id:'bad/id',name:'x.docx',title:'bad'}]);
  assert.equal(records.length,1);assert.equal(records[0].children[0].textContent,'<img onerror=alert(1)>');
  assert.equal(records[0].href,'./editor.html?open=%2Fapi%2Fsamples%2F11111111-1111-4111-8111-111111111111%2Fsample.docx');
});
