// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Reference-compatible score aggregation for the in-process checker.
//!
//! Based on Harunobu 0.2.2 `core/scorer.py` and `core/severity.py`,
//! copyright (c) 2026 Digital Agency of Japan. See ../HARUNOBU-MIT.txt.
//! This module aggregates supplied results only. Callers must distinguish
//! unimplemented rules from passed rules and disclose incomplete coverage.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level { One, Two, Three }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity { Fatal, Critical, Major, Minor, Info }

#[derive(Clone, Debug, PartialEq)]
pub struct RuleResult {
    pub rule_id: String,
    pub passed: bool,
    /// Harunobu excludes results with confidence <= 0 from scoring.
    pub confidence: f64,
    /// Unspecified severity defaults to major, as in Harunobu.
    pub severity: Option<Severity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedRule { pub rule_id: String, pub severity: Severity }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelScore {
    pub level: Level,
    /// None means no rule with positive confidence was checked.
    pub score: Option<u8>,
    pub passed: usize,
    pub total: usize,
    pub forced_zero_rules: Vec<FailedRule>,
    pub failed_rules: Vec<FailedRule>,
}

/// Apply Harunobu's default scoring policy to one level, preserving result order.
/// This is a score of the supplied results, not a claim of complete coverage.
pub fn score_level(level: Level, results: &[RuleResult]) -> LevelScore {
    let mut passed = 0usize;
    let mut total = 0usize;
    let mut deduction = 0u8;
    let mut failed_rules = Vec::new();
    let mut forced_zero_rules = Vec::new();
    for rule in results.iter().filter(|r| r.confidence > 0.0) {
        total += 1;
        if rule.passed { passed += 1; continue; }
        let severity = rule.severity.unwrap_or(Severity::Major);
        let failed = FailedRule { rule_id: rule.rule_id.clone(), severity };
        if severity == Severity::Fatal { forced_zero_rules.push(failed.clone()); }
        else { deduction = deduction.saturating_add(deduction_for(level, severity)); }
        failed_rules.push(failed);
    }
    let score = if total == 0 { None } else if !forced_zero_rules.is_empty() { Some(0) } else { Some(100u8.saturating_sub(deduction)) };
    LevelScore { level, score, passed, total, forced_zero_rules, failed_rules }
}

fn deduction_for(level: Level, severity: Severity) -> u8 {
    match (level, severity) {
        (_, Severity::Fatal | Severity::Info) => 0,
        (Level::One, Severity::Critical) => 10,
        (Level::One, Severity::Major) => 5,
        (Level::One, Severity::Minor) => 2,
        (Level::Two, Severity::Critical) => 40,
        (Level::Two, Severity::Major) => 30,
        (Level::Two, Severity::Minor) => 10,
        (Level::Three, Severity::Critical) => 15,
        (Level::Three, Severity::Major) => 10,
        (Level::Three, Severity::Minor) => 5,
    }
}

/// Aggregate file scores and cross-file check results using Harunobu's default
/// bulk weighting. Callers must ensure the supplied scores cover their inputs.
pub fn score_bulk(files: &[[LevelScore; 3]], bulk_checks: &[RuleResult]) -> u8 {
    if files.is_empty() { return 100; }
    let per_file: Vec<f64> = files.iter().filter_map(|scores| {
        let values: Vec<_> = scores.iter().filter_map(|s| s.score).collect();
        if values.is_empty() { None } else { Some((values.iter().map(|v| f64::from(*v)).sum::<f64>() / values.len() as f64).round_ties_even()) }
    }).collect();
    let mean = if per_file.is_empty() { 100.0 } else { per_file.iter().sum::<f64>() / per_file.len() as f64 };
    let active: Vec<_> = bulk_checks.iter().filter(|c| c.confidence > 0.0).collect();
    let total = if active.is_empty() { mean } else {
        let pass_rate = active.iter().filter(|c| c.passed).count() as f64 / active.len() as f64 * 100.0;
        mean * 0.7 + pass_rate * 0.3
    };
    total.round_ties_even().clamp(0.0, 100.0) as u8
}
