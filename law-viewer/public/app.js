// SPDX-License-Identifier: AGPL-3.0-only

import init, { compare_laws, export_comparison_docx } from "./engine.js";

const $ = id => document.getElementById(id);
const displayButtons = [...document.querySelectorAll("[data-view]")];
const demo = {
  law: "323AC0000000068",
  old: "323AC0000000068_20261001_507AC0000000087",
  new: "323AC0000000068_20270130_508AC0000000076",
};
let selected;
let comparison;
let activeView = "shinkyu";
let generation = 0;
let gitVersion;
const initial = new URLSearchParams(location.search);
let wasm;
const index = fetch("/houritsu-index.json").then(response => response.json()).catch(() => null);

async function request(endpoint, params, asText = false) {
  const response = await fetch(`/api/${endpoint}?${new URLSearchParams(params)}`);
  if (!response.ok) {
    const detail = await response.json().catch(() => ({}));
    throw new Error(detail.error ?? `取得できませんでした（${response.status}）。`);
  }
  return asText ? response.text() : response.json();
}

function clearComparison() {
  comparison = undefined;
  $("toolbar").hidden = true;
  $("git-old").hidden = $("git-new").hidden = true;
  $("output").replaceChildren();
}

function status(message) {
  $("status").textContent = message;
}

async function search(useLink = false) {
  const task = ++generation;
  selected = undefined;
  clearComparison();
  $("results").replaceChildren();
  $("law-title").textContent = "法令を選択";
  $("number").textContent = "";
  $("old").replaceChildren();
  $("new").replaceChildren();
  $("git-source").hidden = $("git-history").hidden = $("egov").hidden = true;
  $("old").disabled = $("new").disabled = $("compare-button").disabled = true;
  status("法令を検索中…");
  try {
    const data = await request("search", { q: $("query").value });
    if (task !== generation) return;
    for (const law of data.laws ?? []) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = law.revision_info.law_title;
      button.title = law.law_info.law_num;
      button.addEventListener("click", () => selectLaw(law, button));
      $("results").append(button);
    }
    status($("results").childElementCount ? "法令を選択してください。" : "一致する法令がありません。");
    if (data.laws?.[0]) {
      const match = useLink ? data.laws.findIndex(law => law.law_info.law_id === initial.get("law")) : 0;
      const position = Math.max(0, match);
      await selectLaw(data.laws[position], $("results").children[position], useLink);
    }
  } catch (error) {
    if (task === generation) status(error.message);
  }
}

async function selectLaw(law, button, useLink = false) {
  const task = ++generation;
  clearComparison();
  const id = law.law_info.law_id;
  selected = law;
  gitVersion = undefined;
  for (const item of $("results").children) item.setAttribute("aria-current", String(item === button));
  $("law-title").textContent = law.revision_info.law_title;
  $("number").textContent = law.law_info.law_num;
  $("egov").href = `https://laws.e-gov.go.jp/law/${id}`;
  $("egov").hidden = false;
  $("git-source").hidden = $("git-history").hidden = true;
  $("old").replaceChildren();
  $("new").replaceChildren();
  $("old").disabled = $("new").disabled = $("compare-button").disabled = true;
  status("改正履歴を取得中…");
  try {
    const [data, git, commits] = await Promise.all([
      request("history", { id }),
      index,
      fetch(`/history/${id.slice(0, 3)}.json`).then(r => r.ok ? r.json() : null).catch(() => null),
    ]);
    if (task !== generation) return;
    if (git?.paths?.[id]) {
      gitVersion = { path: git.paths[id], commits };
      $("git-history").href = `https://github.com/Kohei-Wada/houritsu/commits/${git.commit}/${git.paths[id]}`;
      $("git-source").href = `https://github.com/Kohei-Wada/houritsu/blob/${git.commit}/${git.paths[id]}`;
      $("git-source").hidden = $("git-history").hidden = false;
    }
    const revisions = (data.revisions ?? []).filter(revision => revision.law_revision_id);
    for (const revision of revisions) {
      const option = new Option(
        `${revision.amendment_enforcement_date ?? "施行日未定"}　${revision.amendment_law_title || "制定・初回収録"}`,
        revision.law_revision_id,
      );
      $("old").add(option);
      $("new").add(option.cloneNode(true));
    }
    if (revisions.length < 2) {
      status("比較できる版が二つ以上ありません。");
      return;
    }
    const current = revisions.findIndex(revision => revision.current_revision_status === "CurrentEnforced");
    const newer = Math.min(current >= 0 ? current : 0, revisions.length - 2);
    $("new").selectedIndex = newer;
    $("old").selectedIndex = newer + 1;
    if (id === demo.law && revisions.some(r => r.law_revision_id === demo.new)
        && revisions.some(r => r.law_revision_id === demo.old)) {
      $("new").value = demo.new;
      $("old").value = demo.old;
    }
    if (useLink && initial.get("law") === id) {
      for (const side of ["old", "new"]) {
        if (revisions.some(r => r.law_revision_id === initial.get(side))) {
          $(side).value = initial.get(side);
        }
      }
    }
    $("old").disabled = $("new").disabled = $("compare-button").disabled = false;
    await compare();
  } catch (error) {
    if (task === generation) status(error.message);
  }
}

async function compare() {
  const task = ++generation;
  clearComparison();
  if ($("old").value === $("new").value) {
    status("旧版と新版に異なる版を選択してください。");
    return;
  }
  $("compare-button").disabled = true;
  status("法令XMLを取得し、変更箇所を比較中…");
  try {
    wasm ??= init().catch(error => { wasm = undefined; throw error; });
    const [old, newer] = await Promise.all([
      request("revision", { id: $("old").value }, true),
      request("revision", { id: $("new").value }, true),
      wasm,
    ]);
    if (task !== generation) return;
    comparison = JSON.parse(compare_laws(lawXml(old), lawXml(newer)));
    $("toolbar").hidden = false;
    const params = new URLSearchParams({ q: $("query").value, law: selected.law_info.law_id });
    for (const side of ["old", "new"]) {
      const revision = $(side).value;
      params.set(side, revision);
      const commit = gitVersion?.commits?.[revision];
      if (commit) {
        const link = $(`git-${side}`);
        link.href = `https://github.com/Kohei-Wada/houritsu/blob/${commit}/${gitVersion.path}`;
        link.hidden = false;
      }
    }
    window.history.replaceState(null, "", `?${params}`);
    render("shinkyu");
    status("");
  } catch (error) {
    if (task === generation) status(error.message);
  } finally {
    if (task === generation) $("compare-button").disabled = false;
  }
}

function lawXml(response) {
  const document = new DOMParser().parseFromString(response, "application/xml");
  const law = document.querySelector("Law");
  if (!law || document.querySelector("parsererror")) throw new Error("法令XMLを読み取れません。");
  return new XMLSerializer().serializeToString(law);
}

function appendInline(node, source) {
  const parts = source.split(/(<br>|\+\+.+?\+\+|~~.+?~~|\*\*.+?\*\*)/g);
  for (const part of parts) {
    if (part === "<br>") node.append(document.createElement("br"));
    else {
      const tag = { "++": "u", "~~": "del", "**": "strong" }[part.slice(0, 2)];
      if (tag && part.length > 4) {
        const marked = document.createElement(tag);
        marked.textContent = part.slice(2, -2);
        node.append(marked);
      } else node.append(part);
    }
  }
}

function updateWritingMode() {
  const mode = $("writing-mode").value;
  const vertical = mode === "vertical" || (mode === "auto" && activeView === "shinkyu");
  const output = $("output");
  output.classList.toggle("vertical", vertical);
  $("reading-hint").hidden = !vertical;
  if (vertical) {
    output.tabIndex = 0;
    output.setAttribute("aria-describedby", "reading-hint");
  } else {
    output.removeAttribute("tabindex");
    output.removeAttribute("aria-describedby");
  }
  output.scrollLeft = 0;
  output.scrollTop = 0;
}

function render(view) {
  if (!comparison) return;
  activeView = view;
  for (const button of displayButtons) button.setAttribute("aria-pressed", String(button.dataset.view === view));
  const content = comparison[view];
  const output = $("output");
  output.dataset.view = view;
  updateWritingMode();
  output.replaceChildren();
  let table;
  let frontmatter = content.startsWith("---\n");
  for (const [index, raw] of content.split("\n").entries()) {
    if (frontmatter) {
      if (index > 0 && raw === "---") frontmatter = false;
      continue;
    }
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith("|") && line.endsWith("|")) {
      const columns = line.slice(1, -1).split(/(?<!\\)\|/).map(cell => cell.trim().replaceAll("\\|", "|"));
      if (columns.every(cell => /^:?-+:?$/.test(cell))) continue;
      const header = !table;
      if (header) {
        table = document.createElement("table");
        output.append(table);
      }
      const row = table.insertRow();
      for (const cell of columns) {
        const item = document.createElement(header ? "th" : "td");
        if (header) item.scope = "col";
        appendInline(item, cell);
        row.append(item);
      }
      continue;
    }
    table = undefined;
    const heading = /^(#+)\s+(.*)$/.exec(line);
    const paragraph = document.createElement(heading ? `h${Math.min(6, heading[1].length + 1)}` : "p");
    appendInline(paragraph, heading?.[2] ?? line.replace(/^>\s*/, ""));
    output.append(paragraph);
  }
}

function download(data, extension, type) {
  const filename = selected.law_info.law_id + "-amendment." + extension;
  const url = URL.createObjectURL(new Blob([data], { type }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

$("search").addEventListener("submit", event => { event.preventDefault(); search(); });
$("compare").addEventListener("submit", event => { event.preventDefault(); compare(); });
for (const control of [$("old"), $("new")]) control.addEventListener("change", () => {
  ++generation;
  clearComparison();
  $("compare-button").disabled = false;
  status("比較する版を選択しました。");
});
$("writing-mode").addEventListener("change", updateWritingMode);
for (const button of displayButtons) button.addEventListener("click", () => render(button.dataset.view));
$("comparison-docx").addEventListener("click", () => {
  if (!comparison) return;
  try {
    download(export_comparison_docx(comparison.shinkyu), "docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document");
  } catch (error) { status(error.message); }
});
$("json").addEventListener("click", () => {
  if (comparison) download(JSON.stringify(comparison.amendment_ir, null, 2), "json", "application/json");
});
$("print").addEventListener("click", () => {
  if (comparison) window.print();
});
if (initial.get("q")) $("query").value = initial.get("q").slice(0, 100);
search(true);
