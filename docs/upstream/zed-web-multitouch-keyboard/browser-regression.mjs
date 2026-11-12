// SPDX-License-Identifier: AGPL-3.0-only
// Synthetic browser check against a running Zed gpui_web input example.
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';
const browser = await chromium.launch({executablePath:process.env.IBIS_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true,args:['--disable-webgpu','--use-angle=swiftshader','--disable-gpu-sandbox']});
try {
 const ctx=await browser.newContext({viewport:{width:390,height:844},hasTouch:true,isMobile:true});
 const page=await ctx.newPage(); const failures=[];page.on('pageerror', e=>failures.push(e.message));
 await page.addInitScript(()=>{window.__trace=[]; for(const type of ['pointerdown','pointermove','pointerup','pointercancel']) document.addEventListener(type,e=>{if(e.pointerType==='touch')window.__trace.push({type,id:e.pointerId});},true)});
 await page.goto(process.env.GPUI_UPSTREAM_WEB_URL ?? 'http://127.0.0.1:8799/'); await page.locator('canvas').waitFor({timeout:120000}); await page.waitForTimeout(3000);
 const mirror=page.locator('textarea').first(); const cdp=await ctx.newCDPSession(page);
 const p=(id,x=65,y=45)=>({id,x,y,radiusX:4,radiusY:4,force:1}); const touch=(type,touchPoints)=>cdp.send('Input.dispatchTouchEvent',{type,touchPoints});
 const disarm=()=>mirror.evaluate(el=>el.setAttribute('inputmode','none'));
 const mode=()=>mirror.getAttribute('inputmode');
 async function pair(id,releaseFirst) {
   await touch('touchStart',[p(id)]);await touch('touchStart',[p(id),p(id+1,165)]);await touch('touchMove',[p(id),p(id+1,220)]);
   await touch('touchEnd',[releaseFirst?p(id):p(id+1,220)]);await touch('touchEnd',[]);
 }
 const run=async(name,fn)=>{await fn(); console.log('PASS',name)};
 await run('two contacts do not open keyboard, secondary lifts first',async()=>{await disarm(); await pair(1,false);assert.equal(await mode(),'none')});
 await run('single tap opens keyboard after multi-touch',async()=>{await Promise.all([touch('touchStart',[p(3)]),touch('touchEnd',[])]);assert.equal(await mode(),'text')});
 await run('two contacts do not open keyboard, primary lifts first',async()=>{await disarm();await pair(4,true);assert.equal(await mode(),'none')});
 await run('third contact keeps sequence suppressed',async()=>{await disarm();await touch('touchStart',[p(10)]);await touch('touchStart',[p(10),p(11,165)]);await touch('touchEnd',[p(10)]);await touch('touchStart',[p(11,165),p(12,220)]);await touch('touchEnd',[p(11,165)]);await touch('touchEnd',[]);assert.equal(await mode(),'none')});
 await run('cancelled multi-touch permits later single tap',async()=>{await disarm();await touch('touchStart',[p(20)]);await touch('touchStart',[p(20),p(21,165)]);await touch('touchCancel',[]);assert.equal(await mode(),'none');assert((await page.evaluate(()=>window.__trace)).some(x=>x.type==='pointercancel'));await Promise.all([touch('touchStart',[p(22)]),touch('touchEnd',[])]);assert.equal(await mode(),'text')});
 assert.deepEqual(failures,[]); await ctx.close();
}finally{await browser.close()}
