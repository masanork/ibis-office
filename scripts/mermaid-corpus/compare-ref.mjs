// SPDX-License-Identifier: AGPL-3.0-only
//
// Render every official-docs corpus example with mermaid@12.3.0 and compare
// the result to ibisOffice SVGs (node / edge counts and label presence).
//
// Usage:
//   node scripts/mermaid-corpus/compare-ref.mjs \
//     --corpus crates/ibis-pptx/tests/mermaid/corpus \
//     --ours /tmp/mermaid-svg \
//     --out /tmp/mermaid-js-compare
//
// Missing --ours files are treated as "ibisOffice did not render".

import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer-core";

const here = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_TIMEOUT_MS = 15_000;
const COUNT_TOLERANCE = 0.25;

function parseArgs(argv) {
  const out = {
    corpus: path.join(here, "../../crates/ibis-pptx/tests/mermaid/corpus"),
    ours: "",
    out: "mermaid-js-compare",
    chrome: process.env.CHROME_PATH || process.env.IBIS_CHROME_PATH || "",
    timeout: DEFAULT_TIMEOUT_MS,
  };
  for (let i = 2; i < argv.length; i++) {
    const key = argv[i];
    const val = argv[i + 1];
    if (key === "--corpus") out.corpus = val, i++;
    else if (key === "--ours") out.ours = val, i++;
    else if (key === "--out") out.out = val, i++;
    else if (key === "--chrome") out.chrome = val, i++;
    else if (key === "--timeout-ms") out.timeout = Number(val), i++;
    else if (key === "--help" || key === "-h") {
      console.log("Usage: node compare-ref.mjs [--corpus DIR] [--ours DIR] [--out DIR] [--chrome PATH]");
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${key}`);
    }
  }
  out.corpus = path.resolve(out.corpus);
  out.out = path.resolve(out.out);
  if (out.ours) out.ours = path.resolve(out.ours);
  return out;
}

function findChrome(explicit) {
  if (explicit && fs.existsSync(explicit)) return explicit;
  const candidates = [
    process.env.CHROME_PATH,
    process.env.IBIS_CHROME_PATH,
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium-browser",
    "/usr/bin/chromium",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  ].filter(Boolean);
  for (const c of candidates) {
    if (fs.existsSync(c)) return c;
  }
  return "";
}

function which(cmd) {
  return new Promise((resolve) => {
    const child = spawn("command", ["-v", cmd], { stdio: ["ignore", "pipe", "ignore"] });
    let buf = "";
    child.stdout.on("data", (d) => (buf += d));
    child.on("close", (code) => resolve(code === 0 ? buf.trim() : ""));
  });
}


function startStaticServer(root) {
  const mime = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".mjs": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".json": "application/json",
    ".map": "application/json",
    ".wasm": "application/wasm",
  };
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    let rel = path.normalize(decodeURIComponent(url.pathname)).replace(/^\/+/, "");
    if (!rel || rel === ".") rel = "render.html";
    const file = path.resolve(root, rel);
    if (file !== root && !file.startsWith(root + path.sep)) {
      res.writeHead(403);
      res.end("forbidden");
      return;
    }
    fs.readFile(file, (err, data) => {
      if (err) {
        res.writeHead(404);
        res.end("not found");
        return;
      }
      res.writeHead(200, { "content-type": mime[path.extname(file)] || "application/octet-stream" });
      res.end(data);
    });
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, port: server.address().port });
    });
  });
}

function loadCases(corpusDir) {
  const files = fs.readdirSync(corpusDir).filter((f) => f.endsWith(".mmd")).sort();
  const cases = [];
  for (const file of files) {
    const doc = file.replace(/\.mmd$/, "");
    const text = fs.readFileSync(path.join(corpusDir, file), "utf8");
    let current = null;
    for (const line of text.split(/\n/)) {
      if (line.startsWith("%%%% case ")) {
        current = { doc, id: line.slice("%%%% case ".length).trim(), source: "" };
        cases.push(current);
      } else if (current) {
        current.source += line + "\n";
      }
    }
  }
  return cases;
}

function caseNumber(id) {
  return id.split(/\s+/)[0] || "0";
}

function normalizeLabel(s) {
  return String(s)
    .replace(/\u00a0/g, " ")
    .replace(/[\u200b\u200c\u200d\ufeff]/g, "")
    .replace(/<br\s*\/?>/gi, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function isNoiseLabel(label) {
  const l = normalizeLabel(label);
  if (!l) return true;
  // mermaid sometimes leaks CSS or internal ids into text.
  if (/^#[0-9a-fA-F]{3,8}$/.test(l)) return true;
  if (/^(undefined|null|NaN)$/i.test(l)) return true;
  if (/^mermaid[-:]/i.test(l)) return true;
  // mermaid.js invents gitGraph commit ids (e.g. 0-90fec6f) when the source has none.
  if (/^\d+-[0-9a-f]{7}$/i.test(l)) return true;
  // Font Awesome is not loaded in the comparison page, so mermaid.js falls back to "?".
  if (l === "?") return true;
  return false;
}

function compactLabel(s) {
  return normalizeLabel(s).replace(/\s+/g, "").toLowerCase();
}

function foldedLabel(s) {
  return normalizeLabel(s)
    .toLowerCase()
    .replace(/[«»\[\]<>{}()|_.,:;+*\-`'\"/\\]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function labelPresent(label, ours, joined, compactJoined, foldedJoined) {
  const l = normalizeLabel(label);
  if (!l) return true;
  if (l.length <= 2) {
    return ours.some((o) => o === l || o.split(" ").includes(l));
  }
  if (ours.some((o) => o === l || o.includes(l))) return true;
  // A mermaid label split across text nodes may appear only in the join.
  if (joined.includes(l)) return true;
  // mermaid.js often concatenates wrapped lines without spaces, and uses
  // [Person] / «interface» where we use slightly different markers.
  const c = compactLabel(l);
  if (c.length >= 3 && compactJoined.includes(c)) return true;
  const f = foldedLabel(l);
  if (f.length >= 3 && foldedJoined.includes(f)) return true;
  return false;
}

function countMatches(refLabels, ourLabels) {
  const ours = ourLabels.map(normalizeLabel).filter((l) => !isNoiseLabel(l));
  const joined = ours.join(" ");
  const compactJoined = compactLabel(joined);
  const foldedJoined = foldedLabel(joined);
  const refs = refLabels.map(normalizeLabel).filter((l) => !isNoiseLabel(l));
  const missing = [];
  let found = 0;
  for (const l of refs) {
    if (labelPresent(l, ours, joined, compactJoined, foldedJoined)) found += 1;
    else missing.push(l);
  }
  return { ref: refs.length, found, missing };
}

function countsClose(a, b) {
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  if (hi === 0) return true;
  return hi - lo <= Math.max(1, Math.ceil(COUNT_TOLERANCE * hi));
}

function pct(n, d) {
  if (d === 0) return 0;
  return (100 * n) / d;
}

function fmtPct(n, d) {
  return `${n}/${d} (${pct(n, d).toFixed(1)}%)`;
}

async function withTimeout(promise, ms, label) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`timeout after ${ms}ms (${label})`)), ms);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

async function main() {
  const args = parseArgs(process.argv);
  let chrome = findChrome(args.chrome);
  if (!chrome) chrome = (await which("google-chrome")) || (await which("chromium"));
  if (!chrome) {
    throw new Error("Chrome/Chromium not found; set --chrome or CHROME_PATH");
  }
  const cases = loadCases(args.corpus);
  if (cases.length === 0) {
    throw new Error(`no corpus cases in ${args.corpus}`);
  }
  fs.mkdirSync(args.out, { recursive: true });
  const mermaidSvgDir = path.join(args.out, "mermaid-svg");
  fs.mkdirSync(mermaidSvgDir, { recursive: true });

  const { server, port } = await startStaticServer(here);
  const pageUrl = `http://127.0.0.1:${port}/render.html`;

  const browser = await puppeteer.launch({
    executablePath: chrome,
    headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu", "--font-render-hinting=none"],
  });
  let page = await newPage(browser, pageUrl);

  const results = [];
  let i = 0;
  for (const c of cases) {
    i += 1;
    const n = caseNumber(c.id);
    const key = `${c.doc}-${n}`;
    const ourPath = args.ours ? path.join(args.ours, `${key}.svg`) : "";
    const ourSvg = ourPath && fs.existsSync(ourPath) ? fs.readFileSync(ourPath, "utf8") : "";
    let mermaid = { ok: false, error: "not rendered", svg: "" };
    try {
      mermaid = await withTimeout(
        page.evaluate(async (id, source) => window.renderMermaid(id, source), `ref${i}`, c.source),
        args.timeout,
        key
      );
    } catch (err) {
      mermaid = { ok: false, error: String(err?.message || err), svg: "" };
      // mermaid.js can leave the page in a bad state after a hard failure.
      try {
        await page.close();
      } catch {
        /* ignore */
      }
      page = await newPage(browser, pageUrl);
    }
    if (mermaid.ok && mermaid.svg) {
      fs.writeFileSync(path.join(mermaidSvgDir, `${key}.svg`), mermaid.svg);
    }

    const mermaidMetrics = mermaid.ok
      ? await page.evaluate((svg) => window.svgMetrics(svg, "mermaid"), mermaid.svg)
      : { ok: false, labels: [], nodes: 0, edges: 0, error: mermaid.error };
    const ourMetrics = ourSvg
      ? await page.evaluate((svg) => window.svgMetrics(svg, "ours"), ourSvg)
      : { ok: false, labels: [], nodes: 0, edges: 0, error: ourSvg ? "" : "not rendered" };

    const labels = ourMetrics.ok && mermaidMetrics.ok
      ? countMatches(mermaidMetrics.labels, ourMetrics.labels)
      : { ref: mermaidMetrics.ok ? mermaidMetrics.labels.filter((l) => !isNoiseLabel(l)).length : 0, found: 0, missing: mermaidMetrics.ok ? mermaidMetrics.labels.map(normalizeLabel).filter((l) => !isNoiseLabel(l)) : [] };
    const both = Boolean(mermaid.ok && ourSvg);
    const labelMatch = both && labels.ref === labels.found;
    const structureMatch =
      both &&
      countsClose(mermaidMetrics.nodes || 0, ourMetrics.nodes || 0) &&
      countsClose(mermaidMetrics.edges || 0, ourMetrics.edges || 0);
    results.push({
      doc: c.doc,
      id: c.id,
      key,
      mermaidOk: Boolean(mermaid.ok),
      ourOk: Boolean(ourSvg),
      mermaidError: mermaid.ok ? "" : mermaid.error || mermaidMetrics.error || "",
      ourError: ourSvg ? ourMetrics.error || "" : "not rendered",
      mermaidNodes: mermaidMetrics.nodes || 0,
      mermaidEdges: mermaidMetrics.edges || 0,
      ourNodes: ourMetrics.nodes || 0,
      ourEdges: ourMetrics.edges || 0,
      labelRef: labels.ref,
      labelFound: labels.found,
      labelMissing: labels.missing,
      both,
      labelMatch,
      structureMatch,
      fullMatch: labelMatch && structureMatch,
    });
    if (i % 25 === 0 || i === cases.length) {
      console.error(`rendered ${i}/${cases.length}`);
    }
  }

  await browser.close();
  server.close();
  const report = buildReport(results);
  fs.writeFileSync(path.join(args.out, "report.md"), report.md);
  fs.writeFileSync(path.join(args.out, "report.json"), JSON.stringify(report.json, null, 2) + "\n");
  process.stdout.write(report.md);
  // Non-blocking by default: always exit 0. The workflow marks the job
  // continue-on-error as a belt-and-suspenders measure.
}

async function newPage(browser, pageUrl) {
  const page = await browser.newPage();
  await page.setViewport({ width: 1600, height: 2400, deviceScaleFactor: 1 });
  await page.goto(pageUrl, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.waitForFunction(() => window.__ready === true, { timeout: 60_000 });
  return page;
}

function buildReport(results) {
  const byDoc = new Map();
  for (const r of results) {
    if (!byDoc.has(r.doc)) {
      byDoc.set(r.doc, {
        doc: r.doc,
        total: 0,
        mermaidOk: 0,
        ourOk: 0,
        both: 0,
        labelMatch: 0,
        structureMatch: 0,
        fullMatch: 0,
        missingLabels: 0,
      });
    }
    const s = byDoc.get(r.doc);
    s.total += 1;
    if (r.mermaidOk) s.mermaidOk += 1;
    if (r.ourOk) s.ourOk += 1;
    if (r.both) s.both += 1;
    if (r.labelMatch) s.labelMatch += 1;
    if (r.structureMatch) s.structureMatch += 1;
    if (r.fullMatch) s.fullMatch += 1;
    s.missingLabels += r.labelMissing.length;
  }
  const docs = [...byDoc.values()].sort((a, b) => a.doc.localeCompare(b.doc));
  const tot = docs.reduce(
    (a, s) => {
      for (const k of Object.keys(s)) if (k !== "doc") a[k] += s[k];
      return a;
    },
    { total: 0, mermaidOk: 0, ourOk: 0, both: 0, labelMatch: 0, structureMatch: 0, fullMatch: 0, missingLabels: 0 }
  );

  const ranked = [...docs].sort((a, b) => {
    const ar = a.both ? a.labelMatch / a.both : (a.ourOk ? 0 : 1);
    const br = b.both ? b.labelMatch / b.both : (b.ourOk ? 0 : 1);
    // Worst first among types mermaid.js could render; then by missing labels.
    const aKey = a.mermaidOk === 0 ? 2 : a.ourOk === 0 ? 1 : 0;
    const bKey = b.mermaidOk === 0 ? 2 : b.ourOk === 0 ? 1 : 0;
    if (aKey !== bKey) return aKey - bKey;
    if (ar !== br) return ar - br;
    return b.missingLabels - a.missingLabels;
  });

  let md = "";
  md += "## mermaid.js reference comparison (mermaid@12.3.0)\n\n";
  md += "ibisOffice SVGs are compared to mermaid.js renders of the same official-docs corpus. ";
  md += "Layout is not expected to match. A **label match** means both sides rendered and every mermaid.js label is present in the ibisOffice SVG. ";
  md += "A **structure match** means node and edge counts are within 25% or ±1 (whichever is looser). ";
  md += "This job is report-only and does not fail the workflow. ";
  md += "Typical leftover mismatches are auto-generated labels mermaid.js adds that ibisOffice still does not, or a genuine missing node or edge. ";
  md += "Node and edge counts are normalized: mermaid.js CSS classes and ibisOffice `data-*` attributes are mapped onto the same semantic roles (tasks, commits, slices, series, messages, and so on). Decorative ticks, grids, legends, and fallback shape counts are excluded.\n\n";
  md += `Headline: mermaid.js rendered ${fmtPct(tot.mermaidOk, tot.total)}; ibisOffice rendered ${fmtPct(tot.ourOk, tot.total)}; both ${fmtPct(tot.both, tot.total)}. `;
  md += `Label match ${fmtPct(tot.labelMatch, tot.both)} of both-rendered cases (${fmtPct(tot.labelMatch, tot.total)} of the corpus). `;
  md += `Structure match ${fmtPct(tot.structureMatch, tot.both)} of both-rendered; both metrics ${fmtPct(tot.fullMatch, tot.both)}.\n\n`;
  md += "| doc | total | mermaid.js | ibisOffice | both | label match | of both | structure match | full |\n";
  md += "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n";
  for (const s of docs) {
    md += `| ${s.doc} | ${s.total} | ${s.mermaidOk} | ${s.ourOk} | ${s.both} | ${s.labelMatch} | ${s.both ? pct(s.labelMatch, s.both).toFixed(0) + "%" : "-"} | ${s.structureMatch} | ${s.fullMatch} |\n`;
  }
  md += `| **total** | ${tot.total} | ${tot.mermaidOk} | ${tot.ourOk} | ${tot.both} | ${tot.labelMatch} | ${tot.both ? pct(tot.labelMatch, tot.both).toFixed(0) + "%" : "-"} | ${tot.structureMatch} | ${tot.fullMatch} |\n`;

  md += "\n### Worst-matching diagram types\n\n";
  md += "Ranked by label-match rate among cases both sides rendered (types ibisOffice does not render yet come next; types mermaid.js also fails come last).\n\n";
  md += "| doc | both | label match | missing labels | notes |\n| --- | ---: | ---: | ---: | --- |\n";
  for (const s of ranked.slice(0, 12)) {
    let notes = "";
    if (s.ourOk === 0 && s.mermaidOk > 0) notes = "ibisOffice does not render this type yet";
    else if (s.mermaidOk === 0) notes = "mermaid.js also failed to render";
    else if (s.both && s.labelMatch < s.both) notes = `${s.both - s.labelMatch} cases dropped mermaid.js labels`;
    md += `| ${s.doc} | ${s.both} | ${s.both ? fmtPct(s.labelMatch, s.both) : "0/0"} | ${s.missingLabels} | ${notes} |\n`;
  }

  const offenders = results
    .filter((r) => r.both && r.labelMissing.length > 0)
    .sort((a, b) => b.labelMissing.length - a.labelMissing.length || a.key.localeCompare(b.key))
    .slice(0, 25);
  if (offenders.length) {
    md += "\n<details><summary>Cases with the most missing mermaid.js labels</summary>\n\n";
    md += "| case | mermaid nodes/edges | ours nodes/edges | missing |\n| --- | --- | --- | --- |\n";
    for (const r of offenders) {
      const missing = r.labelMissing.slice(0, 6).map((l) => `\`${l.replace(/\|/g, "\\|")}\``).join(", ");
      const more = r.labelMissing.length > 6 ? ` (+${r.labelMissing.length - 6} more)` : "";
      md += `| ${r.key} | ${r.mermaidNodes}/${r.mermaidEdges} | ${r.ourNodes}/${r.ourEdges} | ${missing}${more} |\n`;
    }
    md += "\n</details>\n";
  }

  const mermaidFails = results.filter((r) => !r.mermaidOk);
  if (mermaidFails.length) {
    md += "\n<details><summary>mermaid.js render failures</summary>\n\n";
    for (const r of mermaidFails) {
      const err = (r.mermaidError || "error").replace(/\s+/g, " ").slice(0, 200);
      md += `- ${r.key}: ${err}\n`;
    }
    md += "\n</details>\n";
  }

  return {
    md,
    json: {
      mermaid: "12.3.0",
      generated: new Date().toISOString(),
      totals: tot,
      docs,
      cases: results,
    },
  };
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
