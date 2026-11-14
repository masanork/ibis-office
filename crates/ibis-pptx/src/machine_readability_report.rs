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
