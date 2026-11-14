export default {
 async fetch(request, env) {
  const url=new URL(request.url);
  if(request.method!=='GET' && request.method!=='HEAD') return new Response('Method not allowed',{status:405});
  const match=url.pathname.match(/^\/(single-br|single|part[0-3])\.wasm$/);
  if(match) {
   const raw=new URL(`/${match[1]}.bin`,url);
   const asset=await env.ASSETS.fetch(new Request(raw,{headers:{'Accept-Encoding':'identity'}}));
   if(!asset.ok || (asset.headers.get('content-encoding') && asset.headers.get('content-encoding')!=='identity')) return new Response('Asset unavailable or encoded unexpectedly',{status:502});
   const headers={'Content-Type':'application/wasm','Content-Encoding':match[1]==='single-br'?'br':'gzip','Cache-Control':'public, max-age=3600','Vary':'Accept-Encoding','Access-Control-Allow-Origin':'*','Cross-Origin-Resource-Policy':'cross-origin'};
   return new Response(request.method==='HEAD'?null:asset.body,{status:200,headers,encodeBody:'manual'});
  }
  const asset=await env.ASSETS.fetch(request);
  const headers=new Headers(asset.headers);
  headers.set('Cross-Origin-Opener-Policy','same-origin');headers.set('Cross-Origin-Embedder-Policy','require-corp');
  return new Response(asset.body,{status:asset.status,headers});
 }
};
