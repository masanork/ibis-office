// SPDX-License-Identifier: AGPL-3.0-only

const base = "https://laws.e-gov.go.jp/api/2/";
const lawId = "[0-9A-Z]{15}";
const revisionId = new RegExp(`^${lawId}_\\d{8}_${lawId}$`);

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    if (!url.pathname.startsWith("/api/")) return env.ASSETS.fetch(request);
    if (request.method !== "GET") return new Response(null, { status: 405 });

    let endpoint;
    let xml = false;
    if (url.pathname === "/api/search") {
      const query = url.searchParams.get("q")?.trim();
      if (!query || query.length > 100) return Response.json({ error: "法令名または番号を入力してください。" }, { status: 400 });
      const search = new URLSearchParams({ limit: "30" });
      search.set(/年.*号/u.test(query) ? "law_num" : "law_title", query);
      if (new RegExp(`^${lawId}$`).test(query)) {
        search.delete("law_title");
        search.set("law_id", query);
      }
      endpoint = `laws?${search}`;
    } else if (url.pathname === "/api/history") {
      const id = url.searchParams.get("id") ?? "";
      if (new RegExp(`^${lawId}$`).test(id)) endpoint = `law_revisions/${id}`;
    } else if (url.pathname === "/api/revision") {
      const id = url.searchParams.get("id") ?? "";
      if (revisionId.test(id)) {
        endpoint = `law_data/${id}?response_format=xml`;
        xml = true;
      }
    }
    if (!endpoint) return Response.json({ error: "取得先を確認してください。" }, { status: 400 });

    const key = new Request(new URL(`/api/cache/${endpoint}`, url.origin));
    const hit = await caches.default.match(key);
    if (hit) return hit;
    try {
      const source = await fetch(base + endpoint, {
        headers: { Accept: xml ? "application/xml" : "application/json" },
        signal: AbortSignal.timeout(15000),
      });
      if (!source.ok) return Response.json({ error: `e-Govから取得できませんでした（${source.status}）。` }, { status: 502 });
      const response = new Response(source.body, { headers: {
        "Content-Type": xml ? "application/xml; charset=utf-8" : "application/json; charset=utf-8",
        "Cache-Control": "public, max-age=3600",
        "X-Content-Type-Options": "nosniff",
      } });
      ctx.waitUntil(caches.default.put(key, response.clone()));
      return response;
    } catch {
      return Response.json({ error: "e-Govへの接続に失敗しました。" }, { status: 502 });
    }
  },
};
