// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Building blocks for Harunobu-compatible JSON reports.
//!
//! Based on Harunobu 0.2.2 output/json_writer.py, copyright (c) 2026
//! Digital Agency of Japan. See ../../ibis-doc-model/HARUNOBU-MIT.txt.
//! Callers must supply the actual violation descriptions and sheet names;
//! incomplete checks cannot be turned into a complete report by this module.
use ibis_doc_model::machine_readability_rule_metadata::rule_metadata;
use ibis_doc_model::machine_readability_rules::{FindingSeverity, RuleEvaluation};
use ibis_doc_model::machine_readability_scoring::{FailedRule, LevelScore, Severity};
use serde_json::{json, Value};

fn severity(value: Severity) -> &'static str {
    match value { Severity::Fatal => "fatal", Severity::Critical => "critical", Severity::Major => "major", Severity::Minor => "minor", Severity::Info => "info" }
}
fn finding_severity(value: FindingSeverity) -> &'static str {
    match value { FindingSeverity::Error => "error", FindingSeverity::Warning => "warning", FindingSeverity::Info => "info" }
}
fn failed_rule(value: &FailedRule) -> Value {
    json!({"ルールID": value.rule_id, "ルール名": rule_metadata(&value.rule_id).map_or("", |m| m.name), "重大度": severity(value.severity)})
}

pub fn level_score_json(score: &LevelScore) -> Value {
    let Some(value) = score.score else { return Value::Null; };
    json!({"スコア": value, "合格": score.passed, "合計": score.total,
        "強制0点": !score.forced_zero_rules.is_empty(),
        "強制0点ルール": score.forced_zero_rules.iter().map(failed_rule).collect::<Vec<_>>(),
        "失敗ルール": score.failed_rules.iter().map(failed_rule).collect::<Vec<_>>()})
}

/// One source entry for every finding, in the same order as the evaluation.
pub struct FindingText<'a> { pub sheet: &'a str, pub description: &'a str }

/// Serialize one result only when the caller supplies every finding's text.
pub fn check_json(check: &RuleEvaluation, message: &str, texts: &[FindingText<'_>]) -> Option<Value> {
    if texts.len() != check.findings.len() { return None; }
    let rule = &check.result;
    let metadata = rule_metadata(&rule.rule_id);
    let violations: Vec<_> = check.findings.iter().zip(texts).map(|(f, t)| json!({
        "シート": t.sheet, "セル範囲": f.location, "説明": t.description, "違反重大度": finding_severity(f.severity)
    })).collect();
    let mut value = json!({"ルールID": rule.rule_id, "ルール名": metadata.map_or("", |m| m.name),
        "ルール説明": metadata.map_or("", |m| m.description), "合否": rule.passed,
        "判定対象外": rule.confidence == 0.0, "重大度": severity(rule.severity.unwrap_or(Severity::Major)),
        "信頼度": rule.confidence, "違反": violations, "メッセージ": message});
    if check.findings.len() > 20 {
        // Python's stable sort preserves the first-seen order when counts tie.
        let mut groups: Vec<(&str, FindingSeverity, Vec<&str>)> = Vec::new();
        for (finding, text) in check.findings.iter().zip(texts) {
            if let Some(group) = groups.iter_mut().find(|g| g.0 == text.sheet && g.1 == finding.severity) {
                group.2.push(&finding.location);
            } else { groups.push((text.sheet, finding.severity, vec![&finding.location])); }
        }
        groups.sort_by_key(|group| std::cmp::Reverse(group.2.len()));
        value["違反グループ"] = json!({"合計件数": check.findings.len(), "グループ": groups.iter().map(|(sheet, level, locations)| json!({
            "シート": sheet, "重大度": finding_severity(*level), "件数": locations.len(), "代表セル範囲": locations.iter().take(3).collect::<Vec<_>>()
        })).collect::<Vec<_>>()});
    }
    Some(value)
}

use ibis_doc_model::machine_readability_layout::{InferredColumnHeader, InferredColumnSchema, InferredColumnType};
use ibis_doc_model::machine_readability_rules::RuleDiagnostics;
use ibis_doc_model::machine_readability_scoring::{score_level, Level, RuleResult};
use ibis_doc_model::sheet::CellRange;

/// Fully supplied report inputs. These types do not claim analysis coverage.
pub struct TableReport<'a> {
    pub range: CellRange,
    pub confidence: f64,
    pub header_rows: &'a [u32],
    pub column_headers: &'a [InferredColumnHeader],
    pub columns: &'a [InferredColumnSchema],
    /// Ordered by level and then by rule execution order.
    pub checks: [&'a [RuleDiagnostics]; 3],
}
pub struct SheetProperty<'a> { pub name: &'a str, pub max_row: u32, pub max_col: u32, pub hidden: bool }
pub struct OtherCell<'a> { pub row: u32, pub column: u32, pub value: Value, pub region_type: &'a str }
pub struct SheetReport<'a> {
    pub name: &'a str,
    pub used_range: &'a str,
    pub hidden: bool,
    pub property: Option<SheetProperty<'a>>,
    pub others: &'a [OtherCell<'a>],
    pub tables: &'a [TableReport<'a>],
}
pub struct FileReport<'a> { pub name: &'a str, pub size: u64, pub format: &'a str, pub sheet_count: usize, pub sheets: &'a [SheetReport<'a>] }

fn column_letters(column: u32) -> String {
    let mut n = u64::from(column) + 1; let mut out = Vec::new();
    while n > 0 { n -= 1; out.push((b'A' + (n % 26) as u8) as char); n /= 26; }
    out.into_iter().rev().collect()
}
fn cell_address(row: u32, column: u32) -> String { format!("{}{}", column_letters(column), u64::from(row) + 1) }
fn range_text(range: CellRange) -> String { format!("{}:{}",cell_address(range.first_row,range.first_column),cell_address(range.last_row,range.last_column)) }
fn column_type(kind: InferredColumnType) -> &'static str { match kind { InferredColumnType::Text=>"text",InferredColumnType::Numeric=>"numeric",InferredColumnType::Date=>"date",InferredColumnType::Mixed=>"mixed",InferredColumnType::Empty=>"empty" } }

/// Assemble the default JSON schema from fully supplied inputs. Invalid ranges
/// or missing diagnostic text are rejected; analysis completeness is the caller's responsibility.
pub fn file_report_json(file: &FileReport<'_>) -> Option<Value> {
    let mut scores: [Vec<RuleResult>; 3] = std::array::from_fn(|_|Vec::new());
    let mut sheets = Vec::new();
    for sheet in file.sheets {
        let mut tables = Vec::new();
        for table in sheet.tables {
            let range = table.range;
            if range.first_row > range.last_row || range.first_column > range.last_column || !table.confidence.is_finite() { return None; }
            let header_range = match (table.header_rows.iter().min(),table.header_rows.iter().max()) {
                (Some(&first),Some(&last)) => Some(range_text(CellRange { first_row:first,last_row:last,..range })), _=>None,
            };
            let mut checks = Vec::new();
            for (index,level) in table.checks.iter().enumerate() {
                for check in *level {
                    let texts: Vec<_> = check.descriptions.iter().map(|description|FindingText { sheet:sheet.name,description }).collect();
                    checks.push(check_json(&check.evaluation,&check.message,&texts)?);
                    scores[index].push(check.evaluation.result.clone());
                }
            }
            let mut result = json!({"範囲":range_text(range),"信頼度":table.confidence,
                "ヘッダー行":table.header_rows.iter().map(|row|u64::from(*row)+1).collect::<Vec<_>>(),
                "ヘッダー範囲":header_range,"列数":u64::from(range.last_column)-u64::from(range.first_column)+1,"評価内容":checks});
            if !table.column_headers.is_empty() { result["columnHeaders"] = json!(table.column_headers.iter().map(|h|json!({"colIndex":u64::from(h.column)+1,"headerRows":h.rows.iter().map(|r|u64::from(*r)+1).collect::<Vec<_>>(),"label":h.label,"isMerged":h.merged})).collect::<Vec<_>>()); }
            if !table.columns.is_empty() { result["columns"] = json!(table.columns.iter().map(|c|json!({"colIndex":u64::from(c.column)+1,"inferredType":column_type(c.kind),"hasHeader":c.has_header})).collect::<Vec<_>>()); }
            tables.push(result);
        }
        let mut result = json!({"シート名":sheet.name,"使用範囲":sheet.used_range,"非表示":sheet.hidden,"評価対象エリア":tables});
        if let Some(property) = &sheet.property { result["sheetProperty"] = json!({"sheetName":property.name,"maxRow":property.max_row,"maxCol":property.max_col,"hidden":property.hidden}); }
        if !sheet.others.is_empty() { result["others"] = json!(sheet.others.iter().map(|o|json!({"row":u64::from(o.row)+1,"col":u64::from(o.column)+1,"value":o.value,"regionType":o.region_type})).collect::<Vec<_>>()); }
        sheets.push(result);
    }
    let levels = [Level::One,Level::Two,Level::Three];
    let level_scores: Vec<_> = levels.into_iter().enumerate().map(|(i,l)|level_score_json(&score_level(l,&scores[i]))).collect();
    Some(json!({"inputファイル名":file.name,"ファイルサイズ":file.size,"フォーマット":file.format,"Sheet数":file.sheet_count,
        "レベル別スコア":{"1":level_scores[0],"2":level_scores[1],"3":level_scores[2]},"sheets":sheets}))
}

/// Assemble a bulk report from complete file inputs and cross-file diagnostics.
pub fn bulk_report_json(files: &[FileReport<'_>], checks: &[RuleDiagnostics]) -> Option<Value> {
    let mut reports = Vec::new(); let mut summaries = Vec::new(); let mut scores = Vec::new();
    for file in files {
        let report = file_report_json(file)?;
        let levels = &report["レベル別スコア"];
        let active: Vec<_> = ["1","2","3"].into_iter().filter_map(|level|levels[level]["スコア"].as_u64()).collect();
        let average = if active.is_empty() { None } else { Some((active.iter().sum::<u64>() as f64 / active.len() as f64).round_ties_even() as u8) };
        if let Some(score) = average { scores.push(score); }
        summaries.push(json!({"ファイル名":file.name,"平均スコア":average}));
        reports.push(report);
    }
    let mut checks_json = serde_json::Map::new();
    for check in checks {
        let texts: Vec<_> = check.descriptions.iter().map(|description|FindingText { sheet:"(bulk)",description }).collect();
        if checks_json.insert(check.evaluation.result.rule_id.clone(),check_json(&check.evaluation,&check.message,&texts)?).is_some() { return None; }
    }
    let average = if scores.is_empty() { 100.0 } else { scores.iter().map(|s|f64::from(*s)).sum::<f64>() / scores.len() as f64 };
    let active: Vec<_> = checks.iter().filter(|check|check.evaluation.result.confidence > 0.0).collect();
    let total = if files.is_empty() { 100 } else if active.is_empty() { average.round_ties_even() as u8 } else {
        let rate = active.iter().filter(|check|check.evaluation.result.passed).count() as f64 / active.len() as f64 * 100.0;
        (average * 0.7 + rate * 0.3).round_ties_even() as u8
    };
    Some(json!({"複数ファイル総合スコア":total,"ファイル数":files.len(),"ファイル横断チェック":checks_json,"ファイル別平均スコア":summaries,"ファイル別結果":reports}))
}
