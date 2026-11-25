// SPDX-License-Identifier: AGPL-3.0-only
//! Mermaid compatibility corpus.
//!
//! Every example from the official mermaid docs (`tests/mermaid/corpus/*.mmd`,
//! MIT, see `tests/mermaid/corpus/LICENSE`) goes through
//! [`ibis_formats::mermaid::render_mermaid`]. The test reports how many of each
//! document's examples render, checks the shape of every rendered result, and
//! fails when a document drops below its floor in `tests/mermaid/thresholds.json`
//! or when a diagram type we do not support stops falling back to a code block.
//!
//! On top of "does it render", `tests/mermaid/features.txt` lists syntax features with a status
//! (`ok`, `gap`, `wontfix`) and a simple text matcher. A case is *clean* when it renders and uses no
//! `gap` feature; `<doc>.clean` floors in the thresholds file keep that number from going down, and
//! the report ranks the gaps by how many examples use them.
//!
//! Set `MERMAID_CORPUS_REPORT=<path>` to also write the report as Markdown.
//! Set `MERMAID_CORPUS_DEMO=<dir>` to write runnable Markdown pages for all cases.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use ibis_formats::mermaid::{MermaidOutput, render_chart_spec_svg, render_mermaid};

struct Case {
    id: String,
    source: String,
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mermaid")
}

fn load_cases(path: &Path) -> Vec<Case> {
    let text = fs::read_to_string(path).expect("read corpus file");
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("%%%% case ") {
            cases.push(Case {
                id: rest.trim().to_string(),
                source: String::new(),
            });
        } else if let Some(case) = cases.last_mut() {
            case.source.push_str(line);
            case.source.push('\n');
        }
    }
    cases
}

/// Minimal reader for the flat `{"doc": number}` threshold file (no serde_json dev-dependency needed).
fn load_thresholds(path: &Path) -> BTreeMap<String, usize> {
    let text = fs::read_to_string(path).expect("read thresholds");
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim().trim_end_matches(',');
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches('"');
        if key.starts_with('_') {
            continue;
        }
        if let Ok(n) = value.trim().parse::<usize>() {
            out.insert(key.to_string(), n);
        }
    }
    out
}

/// Structural checks on a rendered result: `Err` describes the defect.
fn check_output(output: &MermaidOutput) -> Result<(), String> {
    let (bytes, w, h) = match output {
        MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        } => (bytes.clone(), *width_px, *height_px),
        MermaidOutput::Chart(spec) => {
            if spec.series.is_empty() || spec.categories.is_empty() {
                return Err("chart without series or categories".into());
            }
            if spec.series.iter().any(|(_, values)| values.is_empty()) {
                return Err("chart series without values".into());
            }
            let Some((bytes, w, h)) = render_chart_spec_svg(spec) else {
                return Err("chart has no SVG fallback".into());
            };
            (bytes, w, h)
        }
    };
    if w == 0 || h == 0 || w > 20_000 || h > 20_000 {
        return Err(format!("implausible size {w}x{h}"));
    }
    let text = std::str::from_utf8(&bytes).map_err(|e| format!("SVG not UTF-8: {e}"))?;
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("SVG not well-formed: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err("root element is not <svg>".into());
    }
    let drawn = root
        .descendants()
        .filter(|n| {
            matches!(
                n.tag_name().name(),
                "rect" | "path" | "line" | "polyline" | "polygon" | "circle" | "ellipse"
            )
        })
        .count();
    if drawn < 2 {
        return Err("SVG draws (almost) nothing".into());
    }
    if text.contains("NaN") || text.contains("inf\"") {
        return Err("SVG contains non-finite coordinates".into());
    }
    Ok(())
}

/// `MERMAID_CORPUS_DUMP=<dir>` writes every rendered case as `<doc>-<n>.svg` for eyeballing or image diffs.
fn dump_svg(doc: &str, case_id: &str, output: &MermaidOutput) {
    let Ok(dir) = std::env::var("MERMAID_CORPUS_DUMP") else {
        return;
    };
    let bytes = match output {
        MermaidOutput::Svg { bytes, .. } => bytes.clone(),
        MermaidOutput::Chart(spec) => match render_chart_spec_svg(spec) {
            Some((bytes, _, _)) => bytes,
            None => return,
        },
    };
    let n = case_id.split_whitespace().next().unwrap_or("0");
    fs::create_dir_all(&dir).expect("dump dir");
    fs::write(Path::new(&dir).join(format!("{doc}-{n}.svg")), bytes).expect("dump svg");
}

#[derive(Default)]
struct DocStats {
    total: usize,
    rendered: usize,
    clean: usize,
    failures: Vec<String>,
    defects: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Gap,
    WontFix,
}

struct Feature {
    doc: String,
    name: String,
    status: Status,
    matchers: Vec<(String, String)>,
}

fn load_features(path: &Path) -> Vec<Feature> {
    let text = fs::read_to_string(path).expect("read features");
    let mut out: Vec<Feature> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(5, '|').map(str::trim).collect();
        assert_eq!(parts.len(), 5, "bad feature line: {line}");
        let status = match parts[2] {
            "ok" => Status::Ok,
            "gap" => Status::Gap,
            "wontfix" => Status::WontFix,
            other => panic!("bad status {other} in: {line}"),
        };
        assert!(
            matches!(
                parts[3],
                "prefix" | "contains" | "front" | "header" | "config"
            ),
            "bad matcher in: {line}"
        );
        let matcher = (parts[3].to_string(), parts[4].to_string());
        match out
            .iter_mut()
            .find(|f| f.doc == parts[0] && f.name == parts[1])
        {
            Some(feature) => {
                assert!(
                    feature.status == status,
                    "status differs between lines of {}",
                    parts[1]
                );
                feature.matchers.push(matcher);
            }
            None => out.push(Feature {
                doc: parts[0].to_string(),
                name: parts[1].to_string(),
                status,
                matchers: vec![matcher],
            }),
        }
    }
    out
}

/// Leaf keys below the frontmatter `config:` block as dotted paths (`gitGraph.showBranches`,
/// `themeVariables.git0`). Quoted keys are unquoted; block scalars and multi-line quoted strings are
/// skipped like YAML does.
fn config_paths(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.first().is_none_or(|l| l.trim() != "---") {
        return Vec::new();
    }
    let unquote = |s: &str| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
    let mut out = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut i = 1;
    while i < lines.len() && lines[i].trim() != "---" {
        let line = lines[i];
        i += 1;
        let content = line.trim_start();
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        let indent = line.len() - content.len();
        let Some((key, value)) = content.split_once(':') else {
            continue;
        };
        let value = value.split(" #").next().unwrap_or("").trim();
        while stack.last().is_some_and(|(level, _)| *level >= indent) {
            stack.pop();
        }
        if value.is_empty() {
            stack.push((indent, unquote(key)));
            continue;
        }
        for q in ['"', '\''] {
            if value.starts_with(q) && (value.len() == 1 || !value.ends_with(q)) {
                while i < lines.len() && !lines[i].trim_end().ends_with(q) {
                    i += 1;
                }
                i += 1;
            }
        }
        if value.starts_with('|') || value.starts_with('>') {
            while i < lines.len()
                && (lines[i].trim().is_empty()
                    || lines[i].len() - lines[i].trim_start().len() > indent)
            {
                i += 1;
            }
        }
        let mut path: Vec<String> = stack.iter().map(|(_, k)| k.clone()).collect();
        path.push(unquote(key));
        if path.first().is_some_and(|k| k == "config") && path.len() > 1 {
            out.push(path[1..].join("."));
        }
    }
    out
}

/// Frontmatter top-level keys, the header line, and the body lines of a case.
fn split_case(source: &str) -> (Vec<String>, String, Vec<String>) {
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;
    let mut front = Vec::new();
    if lines.first().is_some_and(|l| l.trim() == "---") {
        i = 1;
        while i < lines.len() && lines[i].trim() != "---" {
            if !lines[i].starts_with(char::is_whitespace) {
                front.push(lines[i].trim().to_string());
            }
            i += 1;
        }
        i += 1;
    }
    while i < lines.len() && (lines[i].trim().is_empty() || lines[i].trim().starts_with("%%")) {
        i += 1;
    }
    let header = lines
        .get(i)
        .map(|l| l.trim().to_string())
        .unwrap_or_default();
    let body = lines
        .iter()
        .skip(i + 1)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    (front, header, body)
}

/// `pattern` is a prefix of the dotted `key`; a trailing `$` asks for the whole key
/// (`theme$` is the theme name, not `themeVariables.*`).
fn config_key_matches(key: &str, pattern: &str) -> bool {
    match pattern.strip_suffix('$') {
        Some(exact) => key == exact,
        None => key.starts_with(pattern),
    }
}

struct CaseText<'a> {
    source: &'a str,
    front: &'a [String],
    header: &'a str,
    body: &'a [String],
    /// Every frontmatter `config` leaf.
    config: &'a [String],
    /// The leaves no listed `config` prefix covers (what a `config | *` line matches).
    config_unlisted: &'a [String],
}

fn uses(feature: &Feature, case: &CaseText) -> bool {
    feature
        .matchers
        .iter()
        .any(|(kind, pattern)| match kind.as_str() {
            "prefix" => case.body.iter().any(|l| l.starts_with(pattern.as_str())),
            "contains" => case.source.lines().any(|l| l.contains(pattern.as_str())),
            "front" => case.front.iter().any(|k| k.starts_with(pattern.as_str())),
            "header" => case.header.contains(pattern.as_str()),
            "config" if pattern == "*" => !case.config_unlisted.is_empty(),
            "config" => case.config.iter().any(|k| config_key_matches(k, pattern)),
            _ => false,
        })
}

/// Choose a fence longer than any run of backticks in the original source.
fn append_demo_case(page: &mut String, case: &Case, rendering: &str, limitations: &[String]) {
    let mut longest = 0;
    let mut run = 0;
    for byte in case.source.bytes() {
        run = if byte == b'`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(3.max(longest + 1));
    writeln!(page, "## {}\n\n{rendering}\n", case.id).unwrap();
    for limitation in limitations {
        writeln!(page, "- {limitation}").unwrap();
    }
    if !limitations.is_empty() {
        page.push('\n');
    }
    writeln!(page, "{fence}mermaid\n{}{fence}\n", case.source).unwrap();
}

fn write_web_demo_index(path: &Path, stats: &BTreeMap<String, DocStats>) {
    let total: usize = stats.values().map(|stat| stat.total).sum();
    let rendered: usize = stats.values().map(|stat| stat.rendered).sum();
    let mut html = format!(r#"<!doctype html>
<html lang="ja">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Mermaid 構文デモ集 — Docs</title>
  <link rel="stylesheet" href="../../sorane-home.css">
</head>
<body>
  <header class="topbar"><div class="wrap nav"><a class="brand" href="../../">Docs</a></div></header>
  <main class="wrap">
    <h1>Mermaid 構文デモ集</h1>
    <p>公式例 {total} 件を図種別に開けます。{rendered} 件を描画し、残りはコードで表示します。</p>
    <p><a href="../../editor.html?open=.%2Fsamples%2Fmermaid%2Findex.md">対応状況を Docs で開く</a> · <a href="../../editor.html?open=.%2Fsamples%2Fmermaid-labels.md">英字・日本語・記号の表示例</a> · <a href="LICENSE">MIT ライセンス</a> · <a href="SOURCE.json">出典</a></p>
    <div class="gallery">
"#);
    for (name, stat) in stats {
        assert!(name.bytes().all(|byte| byte.is_ascii_alphanumeric()), "invalid demo file name");
        writeln!(html,
            r#"      <article class="gallery-card"><a class="sample-link" href="../../editor.html?open=.%2Fsamples%2Fmermaid%2F{name}.md">{name} を開く</a> <span>{} / {} 件描画</span> <a href="{name}.md" download>Markdown をダウンロード</a></article>"#,
            stat.rendered, stat.total,
        ).unwrap();
    }
    html.push_str("    </div>\n  </main>\n</body>\n</html>\n");
    fs::write(path.join("web.html"), html).expect("write web demo index");
}

#[test]
fn mermaid_official_corpus() {
    let dir = corpus_dir();
    let thresholds = load_thresholds(&dir.join("thresholds.json"));
    let features = load_features(&dir.join("features.txt"));
    let demo_dir = std::env::var_os("MERMAID_CORPUS_DEMO").map(PathBuf::from);
    if let Some(path) = &demo_dir {
        fs::create_dir_all(path).expect("create demo directory");
        for file in ["LICENSE", "SOURCE.json"] {
            fs::copy(dir.join("corpus").join(file), path.join(file)).expect("copy demo attribution");
        }
    }
    // (cases using it, of those rendered)
    let mut feature_use: Vec<(usize, usize)> = vec![(0, 0); features.len()];
    let mut paths: Vec<PathBuf> = fs::read_dir(dir.join("corpus"))
        .expect("corpus dir")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "mmd"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "empty corpus");

    let mut stats: BTreeMap<String, DocStats> = BTreeMap::new();
    for path in &paths {
        let doc = path.file_stem().unwrap().to_string_lossy().to_string();
        let entry = stats.entry(doc.clone()).or_default();
        let supported = thresholds.contains_key(&doc);
        let mut page = demo_dir.as_ref().map(|_| format!(
            "# {doc}\n\n[目次](index.md) · [ソース](SOURCE.json) · [MIT ライセンス](LICENSE)\n\n"
        ));
        for case in load_cases(path) {
            entry.total += 1;
            let result = catch_unwind(AssertUnwindSafe(|| render_mermaid(&case.source)));
            let mut rendering = "コード表示";
            let rendered = match result {
                Err(_) => {
                    entry.defects.push(format!("{}: panicked", case.id));
                    false
                }
                Ok(None) => {
                    entry.failures.push(case.id.clone());
                    false
                }
                Ok(Some(output)) => match check_output(&output) {
                    Ok(()) => {
                        rendering = match &output {
                            MermaidOutput::Svg { .. } => "SVG",
                            MermaidOutput::Chart(_) => "グラフ（SVG に変換できることも検証済み）",
                        };
                        entry.rendered += 1;
                        dump_svg(&doc, &case.id, &output);
                        true
                    }
                    Err(why) => {
                        entry.defects.push(format!("{}: {why}", case.id));
                        false
                    }
                },
            };
            if !supported {
                if let Some(page) = &mut page {
                    append_demo_case(page, &case, rendering, &[]);
                }
                continue;
            }
            let (front, header, body) = split_case(&case.source);
            let config = config_paths(&case.source);
            let applies = |f: &Feature| f.doc == "*" || f.doc == doc;
            let config_unlisted: Vec<String> = config
                .iter()
                .filter(|path| {
                    !features.iter().filter(|f| applies(f)).any(|f| {
                        f.matchers.iter().any(|(kind, pattern)| {
                            kind == "config" && pattern != "*" && config_key_matches(path, pattern)
                        })
                    })
                })
                .cloned()
                .collect();
            let text = CaseText {
                source: &case.source,
                front: &front,
                header: &header,
                body: &body,
                config: &config,
                config_unlisted: &config_unlisted,
            };
            let mut has_gap = false;
            let mut limitations = Vec::new();
            for (fi, feature) in features.iter().enumerate() {
                if applies(feature) && uses(feature, &text) {
                    feature_use[fi].0 += 1;
                    if rendered {
                        feature_use[fi].1 += 1;
                    }
                    has_gap |= feature.status == Status::Gap;
                    if page.is_some() && feature.status != Status::Ok {
                        let label = if feature.status == Status::Gap { "未実装" } else { "対象外" };
                        if feature.name == "config: other options" {
                            limitations.extend(config_unlisted.iter().map(|key| format!("{label} — `{key}`")));
                        } else {
                            limitations.push(format!("{label} — {}", feature.name));
                        }
                    }
                }
            }
            if rendered && !has_gap {
                entry.clean += 1;
            }
            if let Some(page) = &mut page {
                append_demo_case(page, &case, rendering, &limitations);
            }
        }
        if let (Some(dir), Some(page)) = (&demo_dir, page) {
            fs::write(dir.join(format!("{doc}.md")), page).expect("write demo page");
        }
    }

    let pct = |n: usize, d: usize| {
        if d == 0 {
            0.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };
    let mut report = String::from(
        "## Mermaid corpus (official docs examples)\n\n| doc | total | rendered | rate | floor | clean | clean rate | floor |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    let (mut sup_total, mut sup_ok, mut sup_clean, mut all_total, mut all_ok) = (0, 0, 0, 0, 0);
    let floor_text = |key: &str| {
        thresholds
            .get(key)
            .map_or("-".to_string(), |f| f.to_string())
    };
    for (doc, s) in &stats {
        let supported = thresholds.contains_key(doc);
        report.push_str(&format!(
            "| {doc} | {} | {} | {:.0}% | {} | {} | {} | {} |\n",
            s.total,
            s.rendered,
            pct(s.rendered, s.total),
            floor_text(doc),
            if supported {
                s.clean.to_string()
            } else {
                "-".into()
            },
            if supported {
                format!("{:.0}%", pct(s.clean, s.total))
            } else {
                "-".into()
            },
            floor_text(&format!("{doc}.clean")),
        ));
        all_total += s.total;
        all_ok += s.rendered;
        if supported {
            sup_total += s.total;
            sup_ok += s.rendered;
            sup_clean += s.clean;
        }
    }
    report.push_str(&format!(
        "\nSupported diagram types: rendered {sup_ok}/{sup_total} ({:.1}%), clean {sup_clean}/{sup_total} ({:.1}%). All docs: rendered {all_ok}/{all_total} ({:.1}%).\n",
        pct(sup_ok, sup_total),
        pct(sup_clean, sup_total),
        pct(all_ok, all_total)
    ));
    let mut ranked: Vec<usize> = (0..features.len())
        .filter(|&i| feature_use[i].0 > 0)
        .collect();
    ranked.sort_by_key(|&i| {
        (
            features[i].status != Status::Gap,
            std::cmp::Reverse(feature_use[i].0),
        )
    });
    report.push_str("\n### Features (gaps first, by number of examples using them)\n\n| doc | feature | status | examples |\n| --- | --- | --- | ---: |\n");
    for i in ranked {
        let f = &features[i];
        let status = match f.status {
            Status::Ok => "ok",
            Status::Gap => "gap",
            Status::WontFix => "wontfix",
        };
        report.push_str(&format!(
            "| {} | {} | {status} | {} |\n",
            f.doc, f.name, feature_use[i].0
        ));
    }
    let mut detail = String::new();
    for (doc, s) in &stats {
        if thresholds.contains_key(doc) && !s.failures.is_empty() {
            detail.push_str(&format!(
                "- {doc}: not rendered: {}\n",
                s.failures.join(", ")
            ));
        }
        for d in &s.defects {
            detail.push_str(&format!("- {doc}: defect: {d}\n"));
        }
    }
    if !detail.is_empty() {
        report.push_str("\n<details><summary>Failures</summary>\n\n");
        report.push_str(&detail);
        report.push_str("\n</details>\n");
    }
    if let Some(path) = demo_dir {
        let mut index = String::from(
            "# Mermaid 構文デモ集\n\n[ソース](SOURCE.json) · [MIT ライセンス](LICENSE)\n\n             各ページを Docs で開くと公式例を表示できます。SVG とグラフの区分はレンダラーの出力です。             対応範囲の注記には構文の簡易マッチを使っています。個々の描画や配置の一致を保証するものではありません。\n\n             | 図種 | 描画例 | 全例 |\n| --- | ---: | ---: |\n",
        );
        for (doc, stats) in &stats {
            writeln!(index, "| [{doc}]({doc}.md) | {} | {} |", stats.rendered, stats.total).unwrap();
        }
        index.push_str("\n## 登録済みの構文と設定\n\n| 図種 | 項目 | 状態 | マッチした公式例 |\n| --- | --- | --- | ---: |\n");
        for (feature, (count, _)) in features.iter().zip(&feature_use) {
            let status = match feature.status {
                Status::Ok => "対応",
                Status::Gap => "未実装",
                Status::WontFix => "対象外",
            };
            let name = feature.name.replace('|', "\\|");
            writeln!(index, "| {} | {name} | {status} | {count} |", feature.doc).unwrap();
        }
        fs::write(path.join("index.md"), index).expect("write demo index");
        write_web_demo_index(&path, &stats);
    }
    println!("{report}");
    if let Ok(path) = std::env::var("MERMAID_CORPUS_REPORT") {
        fs::write(path, &report).expect("write report");
    }

    let mut problems = Vec::new();
    for (doc, s) in &stats {
        problems.extend(s.defects.iter().map(|d| format!("{doc}: {d}")));
        if let Some(&floor) = thresholds.get(&format!("{doc}.clean"))
            && s.clean < floor
        {
            problems.push(format!(
                "{doc}: {} clean, below the floor of {floor}",
                s.clean
            ));
        }
        match thresholds.get(doc) {
            Some(&floor) if s.rendered < floor => {
                problems.push(format!("{doc}: {} rendered, below the floor of {floor}", s.rendered))
            }
            None if s.rendered > 0 => problems.push(format!(
                "{doc}: {} examples rendered for a diagram type without a floor; add it to thresholds.json",
                s.rendered
            )),
            _ => {}
        }
    }
    for doc in thresholds.keys() {
        let doc = doc.strip_suffix(".clean").unwrap_or(doc);
        if !stats.contains_key(doc) {
            problems.push(format!(
                "{doc}: in thresholds.json but missing from the corpus"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "Mermaid corpus regressions:\n{}",
        problems.join("\n")
    );
}

#[test]
fn bundled_mermaid_showcase_renders_all_diagrams() {
    let showcase_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../www/samples/mermaid-showcase.md");
    let text = fs::read_to_string(&showcase_path).expect("read www/samples/mermaid-showcase.md");
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(buf) = &mut current {
            if line.trim() == "```" {
                blocks.push(current.take().unwrap());
            } else {
                buf.push_str(line);
                buf.push('\n');
            }
        } else if line.trim() == "```mermaid" {
            current = Some(String::new());
        }
    }
    assert!(blocks.len() >= 10, "expected at least 10 mermaid blocks, got {}", blocks.len());
    for (idx, source) in blocks.iter().enumerate() {
        let output = render_mermaid(source)
            .unwrap_or_else(|| panic!("mermaid-showcase block #{} failed to render:\n{source}", idx + 1));
        check_output(&output)
            .unwrap_or_else(|err| panic!("mermaid-showcase block #{} invalid ({err}):\n{source}", idx + 1));
    }
}

#[test]
fn bundled_web_samples_open_without_error() {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../www/samples");

    let csv = fs::read(samples.join("standard-linkage.csv")).expect("read standard-linkage.csv");
    let opened_csv = ibis_formats::open(csv).expect("standard-linkage.csv must open cleanly");
    let wb = opened_csv.presentation.workbook.as_ref().expect("workbook");
    assert_eq!(wb.sheets.len(), 1);
    assert!(!opened_csv.presentation.slides.is_empty());

    assert!(samples.join("visio-multi-page-handbook.vsdx").is_file(), "visio-multi-page-handbook.vsdx must exist");

    let name = "ibis-office-wasm.sbom.cdx.json";
    let json = fs::read(samples.join(name)).expect("read bom sample");
    let opened = ibis_formats::sbom_review::open_table_with_format((name, &json))
        .unwrap_or_else(|err| panic!("{name} failed: {err}"))
        .unwrap_or_else(|| panic!("{name} was not recognized as CycloneDX"));
    assert!(opened.0.presentation.workbook.is_some(), "{name} should produce a workbook");
}
