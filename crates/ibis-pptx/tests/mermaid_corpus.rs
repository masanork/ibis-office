// SPDX-License-Identifier: AGPL-3.0-only
//! Mermaid compatibility corpus.
//!
//! Every example from the official mermaid docs (`tests/mermaid/corpus/*.mmd`,
//! MIT, see `tests/mermaid/corpus/LICENSE`) goes through
//! [`ibis_pptx::mermaid::render_mermaid`]. The test reports how many of each
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

use std::collections::BTreeMap;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use ibis_pptx::mermaid::{MermaidOutput, render_chart_spec_svg, render_mermaid};

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
            matches!(parts[3], "prefix" | "contains" | "front" | "header"),
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

fn uses(feature: &Feature, source: &str, front: &[String], header: &str, body: &[String]) -> bool {
    feature
        .matchers
        .iter()
        .any(|(kind, pattern)| match kind.as_str() {
            "prefix" => body.iter().any(|l| l.starts_with(pattern.as_str())),
            "contains" => source.lines().any(|l| l.contains(pattern.as_str())),
            "front" => front.iter().any(|k| k.starts_with(pattern.as_str())),
            "header" => header.contains(pattern.as_str()),
            _ => false,
        })
}

#[test]
fn mermaid_official_corpus() {
    let dir = corpus_dir();
    let thresholds = load_thresholds(&dir.join("thresholds.json"));
    let features = load_features(&dir.join("features.txt"));
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
        for case in load_cases(path) {
            entry.total += 1;
            let result = catch_unwind(AssertUnwindSafe(|| render_mermaid(&case.source)));
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
                continue;
            }
            let (front, header, body) = split_case(&case.source);
            let mut has_gap = false;
            for (fi, feature) in features.iter().enumerate() {
                if (feature.doc == "*" || feature.doc == doc)
                    && uses(feature, &case.source, &front, &header, &body)
                {
                    feature_use[fi].0 += 1;
                    if rendered {
                        feature_use[fi].1 += 1;
                    }
                    has_gap |= feature.status == Status::Gap;
                }
            }
            if rendered && !has_gap {
                entry.clean += 1;
            }
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
