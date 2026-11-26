// SPDX-License-Identifier: AGPL-3.0-only
//! AI fine-tuning (SFT) and RLHF / preference / feedback dataset detection,
//! streaming statistics, quality checks, and visual block generation.

use std::collections::BTreeMap;

use serde_json::Value;

use super::visual::{
    CalloutTone, PreviewBlock, PreviewSummary, TableCellValue, ValueKind, build_json_blocks,
};
use super::{Field, push};

const MAX_RENDERED_RECORDS: usize = 25;
const MAX_SCAN_LINES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AiDatasetKind {
    SftOpenAi,
    SftShareGpt,
    SftAlpaca,
    SftGemini,
    RlhfDpo,
    RlhfKto,
    RlhfReward,
}

impl AiDatasetKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            AiDatasetKind::SftOpenAi => "AI SFT (OpenAI / ChatML)",
            AiDatasetKind::SftShareGpt => "AI SFT (ShareGPT)",
            AiDatasetKind::SftAlpaca => "AI SFT (Alpaca / Instruction)",
            AiDatasetKind::SftGemini => "AI SFT (Gemini / Vertex AI)",
            AiDatasetKind::RlhfDpo => "AI RLHF / DPO Preference Pairs",
            AiDatasetKind::RlhfKto => "AI RLHF / KTO Binary Feedback",
            AiDatasetKind::RlhfReward => "AI RLHF / Reward & Rubric Feedback",
        }
    }
}

pub(super) fn detect_record_kind(val: &Value) -> Option<AiDatasetKind> {
    let Value::Object(map) = val else {
        return None;
    };
    // Preference / RLHF first so `chosen`/`rejected` with message arrays aren't misclassified as SFT.
    if map.contains_key("chosen") && map.contains_key("rejected") {
        return Some(AiDatasetKind::RlhfDpo);
    }
    if map.contains_key("label")
        && (map.contains_key("completion") || map.contains_key("response"))
        && map.contains_key("prompt")
    {
        return Some(AiDatasetKind::RlhfKto);
    }
    if (map.contains_key("score")
        || map.contains_key("reward")
        || map.contains_key("rating")
        || map.contains_key("ranking")
        || map.contains_key("rubric")
        || map.contains_key("critique"))
        && (map.contains_key("prompt")
            || map.contains_key("response")
            || map.contains_key("candidates"))
    {
        return Some(AiDatasetKind::RlhfReward);
    }
    if map.get("messages").is_some_and(Value::is_array) {
        return Some(AiDatasetKind::SftOpenAi);
    }
    if map.get("conversations").is_some_and(Value::is_array) {
        return Some(AiDatasetKind::SftShareGpt);
    }
    if map.get("contents").is_some_and(Value::is_array) {
        return Some(AiDatasetKind::SftGemini);
    }
    if map.contains_key("instruction") && map.contains_key("output") {
        return Some(AiDatasetKind::SftAlpaca);
    }
    None
}

pub(super) fn try_build_ai_blocks_from_value(
    val: &Value,
    byte_size: usize,
    fields: &[Field],
) -> Option<(PreviewSummary, Vec<PreviewBlock>)> {
    let records: Vec<&Value> = match val {
        Value::Array(arr) if !arr.is_empty() => arr.iter().collect(),
        Value::Object(_) => vec![val],
        _ => return None,
    };
    if let Some(res) =
        super::personas_evals::try_build_persona_eval_preview(&records, byte_size, fields.len())
    {
        return Some(res);
    }
    let primary_kind = records.iter().find_map(|r| detect_record_kind(r))?;
    Some(build_ai_dataset_preview(
        &records,
        primary_kind,
        byte_size,
        fields.len(),
        Vec::new(),
    ))
}

/// Parse `.jsonl` / `.ndjson` streamingly: compute full-file statistics & quality warnings
/// across all lines (up to `MAX_SCAN_LINES`), while keeping rendered fields/blocks bounded.
pub(super) fn read_jsonl(
    text: &str,
    byte_size: usize,
) -> Result<(Vec<Field>, PreviewSummary, Vec<PreviewBlock>), String> {
    let mut parsed_records: Vec<(usize, Value)> = Vec::new();
    let mut malformed_lines: Vec<usize> = Vec::new();
    let mut total_lines = 0usize;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        total_lines += 1;
        if total_lines > MAX_SCAN_LINES {
            break;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => parsed_records.push((line_no, v)),
            Err(_) => malformed_lines.push(line_no),
        }
    }

    if parsed_records.is_empty() && malformed_lines.is_empty() {
        return Err("Empty JSONL file".into());
    }
    if parsed_records.is_empty() {
        return Err("Invalid JSONL: no valid JSON records found".into());
    }

    let mut fields = Vec::new();
    for (line_no, rec) in parsed_records.iter().take(MAX_RENDERED_RECORDS) {
        if fields.len() >= super::MAX_FIELDS {
            break;
        }
        let _ = super::json_fields(
            rec,
            &format!("/{line_no}"),
            &format!("line {line_no}"),
            1,
            &mut fields,
        );
    }

    let refs: Vec<&Value> = parsed_records.iter().map(|(_, v)| v).collect();
    if let Some((summary, blocks)) =
        super::personas_evals::try_build_persona_eval_preview(&refs, byte_size, fields.len())
    {
        return Ok((fields, summary, blocks));
    }
    if let Some(primary_kind) = refs.iter().find_map(|r| detect_record_kind(r)) {
        let (summary, mut blocks) = build_ai_dataset_preview(
            &refs,
            primary_kind,
            byte_size,
            fields.len(),
            malformed_lines.clone(),
        );
        // Insert a compact Record Index overview table right after Statistics for multi-record JSONL.
        if parsed_records.len() > 1 {
            let index_table = build_jsonl_index_table(&parsed_records, &malformed_lines);
            let insert_at = blocks
                .iter()
                .position(|b| matches!(b, PreviewBlock::Table { title: Some(t), .. } if t.ends_with("Statistics")))
                .map_or(0, |p| p + 1);
            blocks.insert(insert_at, index_table);
        }
        return Ok((fields, summary, blocks));
    }

    // Generic JSONL fallback: render summary + Record Index + first N records.
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();
    if !malformed_lines.is_empty() {
        let msg = format!(
            "{} malformed JSONL line(s): {:?}",
            malformed_lines.len(),
            &malformed_lines[..malformed_lines.len().min(5)]
        );
        warnings.push(msg.clone());
        blocks.push(PreviewBlock::Callout {
            tone: CalloutTone::Warning,
            title: "Malformed JSONL lines".into(),
            body: msg,
        });
    }
    if parsed_records.len() > 1 {
        blocks.push(build_jsonl_index_table(&parsed_records, &malformed_lines));
    }
    let slice_val = Value::Array(
        parsed_records
            .iter()
            .take(MAX_RENDERED_RECORDS)
            .map(|(_, v)| v.clone())
            .collect(),
    );
    build_json_blocks(&slice_val, "", "records", 0, &mut blocks)?;
    let summary = super::visual::summarize_blocks(
        byte_size,
        fields.len(),
        &blocks,
        Some(format!("JSONL ({} records)", parsed_records.len())),
        warnings,
    );
    Ok((fields, summary, blocks))
}

fn build_jsonl_index_table(
    parsed_records: &[(usize, Value)],
    malformed_lines: &[usize],
) -> PreviewBlock {
    const MAX_INDEX_ROWS: usize = 50;
    let mut rows: Vec<Vec<TableCellValue>> = Vec::new();
    for (line_no, val) in parsed_records.iter().take(MAX_INDEX_ROWS) {
        let (preview_txt, detail_txt, status_txt, status_kind) = summarize_jsonl_record(val);
        rows.push(vec![
            TableCellValue {
                text: format!("L{line_no}"),
                kind: ValueKind::Number,
            },
            TableCellValue {
                text: preview_txt,
                kind: ValueKind::String,
            },
            TableCellValue {
                text: detail_txt,
                kind: ValueKind::Attribute,
            },
            TableCellValue {
                text: status_txt,
                kind: status_kind,
            },
        ]);
    }
    for &bad_line in malformed_lines.iter().take(10) {
        rows.push(vec![
            TableCellValue {
                text: format!("L{bad_line}"),
                kind: ValueKind::Number,
            },
            TableCellValue {
                text: "(parse error)".into(),
                kind: ValueKind::Null,
            },
            TableCellValue {
                text: "-".into(),
                kind: ValueKind::Empty,
            },
            TableCellValue {
                text: "ERR: malformed JSON".into(),
                kind: ValueKind::Boolean,
            },
        ]);
    }
    PreviewBlock::Table {
        title: Some(format!("Record Index ({} records)", parsed_records.len())),
        depth: 0,
        headers: vec![
            "Line".into(),
            "Preview".into(),
            "Shape / Turns".into(),
            "Status".into(),
        ],
        rows,
    }
}

fn summarize_jsonl_record(val: &Value) -> (String, String, String, ValueKind) {
    let snippet = |s: &str| -> String {
        let single = s.replace(['\r', '\n'], " ");
        let mut out: String = single.chars().take(56).collect();
        if single.chars().count() > 56 {
            out.push('…');
        }
        out
    };
    if let Some(kind) = detect_record_kind(val) {
        match kind {
            AiDatasetKind::SftOpenAi => {
                let msgs = val.get("messages").and_then(Value::as_array);
                let n = msgs.map_or(0, Vec::len);
                let first_user = msgs
                    .and_then(|arr| {
                        arr.iter()
                            .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
                            .or_else(|| arr.first())
                    })
                    .map(|m| extract_text(m.get("content")))
                    .unwrap_or_default();
                let empty_asst = msgs.is_some_and(|arr| {
                    arr.iter().any(|m| {
                        m.get("role").and_then(Value::as_str) == Some("assistant")
                            && extract_text(m.get("content")).trim().is_empty()
                            && m.get("tool_calls").is_none()
                    })
                });
                return (
                    snippet(&first_user),
                    format!("{n} turns"),
                    if empty_asst {
                        "WARN: empty assistant".into()
                    } else {
                        "OK".into()
                    },
                    if empty_asst {
                        ValueKind::Boolean
                    } else {
                        ValueKind::String
                    },
                );
            }
            AiDatasetKind::RlhfDpo => {
                let prompt = extract_text(val.get("prompt"));
                let chosen = format_preference_side(val.get("chosen"));
                let rejected = format_preference_side(val.get("rejected"));
                let dup = chosen.trim() == rejected.trim() && !chosen.trim().is_empty();
                let empty = chosen.trim().is_empty() || rejected.trim().is_empty();
                let status = if dup {
                    ("WARN: chosen == rejected".into(), ValueKind::Boolean)
                } else if empty {
                    ("WARN: empty side".into(), ValueKind::Boolean)
                } else {
                    ("OK".into(), ValueKind::String)
                };
                return (
                    snippet(if prompt.is_empty() { &chosen } else { &prompt }),
                    "DPO pair".into(),
                    status.0,
                    status.1,
                );
            }
            AiDatasetKind::RlhfKto => {
                let prompt = extract_text(val.get("prompt"));
                let lbl = val.get("label").and_then(Value::as_bool);
                return (
                    snippet(&prompt),
                    format!("KTO ({})", lbl.map_or("?".to_string(), |b| b.to_string())),
                    "OK".into(),
                    ValueKind::String,
                );
            }
            _ => {}
        }
    }
    let summary = match val {
        Value::Object(map) => {
            let keys: Vec<&str> = map.keys().take(4).map(String::as_str).collect();
            (
                snippet(&keys.join(", ")),
                format!("{{{} keys}}", map.len()),
                "OK".into(),
                ValueKind::String,
            )
        }
        Value::Array(arr) => (
            snippet(&val.to_string()),
            format!("[{} items]", arr.len()),
            "OK".into(),
            ValueKind::String,
        ),
        other => (
            snippet(&other.to_string()),
            "scalar".into(),
            "OK".into(),
            ValueKind::String,
        ),
    };
    summary
}

fn build_ai_dataset_preview(
    records: &[&Value],
    primary_kind: AiDatasetKind,
    byte_size: usize,
    field_count: usize,
    malformed_lines: Vec<usize>,
) -> (PreviewSummary, Vec<PreviewBlock>) {
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();
    let mut role_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_turns = 0usize;
    let mut total_chars = 0usize;
    let mut empty_responses = 0usize;
    let mut identical_pairs = 0usize;
    let mut kto_true = 0usize;
    let mut kto_false = 0usize;
    let mut scores: Vec<f64> = Vec::new();

    for rec in records {
        match primary_kind {
            AiDatasetKind::SftOpenAi => {
                if let Some(msgs) = rec.get("messages").and_then(Value::as_array) {
                    for m in msgs {
                        let role = m
                            .get("role")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let text = extract_text(m.get("content"));
                        total_turns += 1;
                        total_chars += text.chars().count();
                        *role_counts.entry(role.clone()).or_default() += 1;
                        if role == "assistant"
                            && text.trim().is_empty()
                            && m.get("tool_calls").is_none()
                        {
                            empty_responses += 1;
                        }
                    }
                }
            }
            AiDatasetKind::SftShareGpt => {
                if let Some(convs) = rec.get("conversations").and_then(Value::as_array) {
                    for c in convs {
                        let role = c
                            .get("from")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let text = extract_text(c.get("value"));
                        total_turns += 1;
                        total_chars += text.chars().count();
                        *role_counts.entry(role.clone()).or_default() += 1;
                        if matches!(role.as_str(), "gpt" | "assistant") && text.trim().is_empty() {
                            empty_responses += 1;
                        }
                    }
                }
            }
            AiDatasetKind::SftAlpaca => {
                let instr = extract_text(rec.get("instruction"));
                let out = extract_text(rec.get("output"));
                total_turns += 2;
                total_chars += instr.chars().count() + out.chars().count();
                if out.trim().is_empty() {
                    empty_responses += 1;
                }
            }
            AiDatasetKind::SftGemini => {
                if let Some(contents) = rec.get("contents").and_then(Value::as_array) {
                    for c in contents {
                        let role = c
                            .get("role")
                            .and_then(Value::as_str)
                            .unwrap_or("user")
                            .to_string();
                        let text = extract_gemini_parts(c.get("parts"));
                        total_turns += 1;
                        total_chars += text.chars().count();
                        *role_counts.entry(role.clone()).or_default() += 1;
                        if role == "model" && text.trim().is_empty() {
                            empty_responses += 1;
                        }
                    }
                }
            }
            AiDatasetKind::RlhfDpo => {
                let chosen = format_preference_side(rec.get("chosen"));
                let rejected = format_preference_side(rec.get("rejected"));
                total_chars += chosen.chars().count() + rejected.chars().count();
                if chosen.trim().is_empty() || rejected.trim().is_empty() {
                    empty_responses += 1;
                }
                if chosen.trim() == rejected.trim() && !chosen.trim().is_empty() {
                    identical_pairs += 1;
                }
            }
            AiDatasetKind::RlhfKto => {
                let comp = extract_text(rec.get("completion").or_else(|| rec.get("response")));
                total_chars += comp.chars().count();
                if comp.trim().is_empty() {
                    empty_responses += 1;
                }
                match rec.get("label").and_then(Value::as_bool) {
                    Some(true) => kto_true += 1,
                    Some(false) => kto_false += 1,
                    None => {}
                }
            }
            AiDatasetKind::RlhfReward => {
                let resp = extract_text(rec.get("response"));
                total_chars += resp.chars().count();
                if let Some(s) = rec
                    .get("score")
                    .or_else(|| rec.get("reward"))
                    .or_else(|| rec.get("rating"))
                    .and_then(Value::as_f64)
                {
                    scores.push(s);
                }
            }
        }
    }

    if !malformed_lines.is_empty() {
        let msg = format!(
            "{} malformed JSONL line(s): {:?}",
            malformed_lines.len(),
            &malformed_lines[..malformed_lines.len().min(5)]
        );
        warnings.push(msg.clone());
        blocks.push(PreviewBlock::Callout {
            tone: CalloutTone::Danger,
            title: "Malformed JSONL lines".into(),
            body: msg,
        });
    }
    if identical_pairs > 0 {
        let msg = format!("{identical_pairs} record(s) have identical chosen and rejected text");
        warnings.push(msg.clone());
        blocks.push(PreviewBlock::Callout {
            tone: CalloutTone::Warning,
            title: "Identical preference pair".into(),
            body: msg,
        });
    }
    if empty_responses > 0 {
        let msg = format!("{empty_responses} record(s) have empty assistant/completion output");
        warnings.push(msg.clone());
        blocks.push(PreviewBlock::Callout {
            tone: CalloutTone::Warning,
            title: "Empty response detected".into(),
            body: msg,
        });
    }

    // Dataset statistics table
    let mut stat_rows = vec![
        vec![
            TableCellValue {
                text: "Records".into(),
                kind: ValueKind::Attribute,
            },
            TableCellValue {
                text: records.len().to_string(),
                kind: ValueKind::Number,
            },
        ],
        vec![
            TableCellValue {
                text: "Total chars".into(),
                kind: ValueKind::Attribute,
            },
            TableCellValue {
                text: total_chars.to_string(),
                kind: ValueKind::Number,
            },
        ],
    ];
    if total_turns > 0 {
        stat_rows.push(vec![
            TableCellValue {
                text: "Turns / Roles".into(),
                kind: ValueKind::Attribute,
            },
            TableCellValue {
                text: if role_counts.is_empty() {
                    format!("{total_turns} turns")
                } else {
                    let roles: Vec<String> = role_counts
                        .iter()
                        .map(|(k, v)| format!("{k}: {v}"))
                        .collect();
                    format!("{total_turns} turns ({})", roles.join(", "))
                },
                kind: ValueKind::String,
            },
        ]);
    }
    if kto_true + kto_false > 0 {
        stat_rows.push(vec![
            TableCellValue {
                text: "KTO Labels".into(),
                kind: ValueKind::Attribute,
            },
            TableCellValue {
                text: format!("true: {kto_true} / false: {kto_false}"),
                kind: ValueKind::Boolean,
            },
        ]);
    }
    if !scores.is_empty() {
        let min = scores.iter().copied().fold(f64::INFINITY, f64::min);
        let max = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let avg = scores.iter().sum::<f64>() / scores.len() as f64;
        stat_rows.push(vec![
            TableCellValue {
                text: "Score / Reward".into(),
                kind: ValueKind::Attribute,
            },
            TableCellValue {
                text: format!("avg {avg:.2} (min {min:.2}, max {max:.2})"),
                kind: ValueKind::Number,
            },
        ]);
    }
    blocks.push(PreviewBlock::Table {
        title: Some(format!("{} — Statistics", primary_kind.label())),
        depth: 0,
        headers: vec!["Metric".into(), "Value".into()],
        rows: stat_rows,
    });

    if records.len() > MAX_RENDERED_RECORDS {
        blocks.push(PreviewBlock::Callout {
            tone: CalloutTone::Info,
            title: "Preview truncated".into(),
            body: format!(
                "Showing first {} of {} records (full statistics computed across all records).",
                MAX_RENDERED_RECORDS,
                records.len()
            ),
        });
    }

    // Render first MAX_RENDERED_RECORDS records using domain-specific blocks.
    for (idx, rec) in records.iter().take(MAX_RENDERED_RECORDS).enumerate() {
        let rec_no = idx + 1;
        let kind = detect_record_kind(rec).unwrap_or(primary_kind);
        match kind {
            AiDatasetKind::SftOpenAi => {
                blocks.push(PreviewBlock::Section {
                    title: format!("Record #{rec_no}"),
                    badge: Some("OpenAI / ChatML".into()),
                    depth: 0,
                });
                if let Some(msgs) = rec.get("messages").and_then(Value::as_array) {
                    for m in msgs {
                        let role = m
                            .get("role")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let content = extract_text(m.get("content"));
                        let tool_calls = m
                            .get("tool_calls")
                            .map(|tc| serde_json::to_string_pretty(tc).unwrap_or_else(|_| tc.to_string()));
                        blocks.push(PreviewBlock::ChatTurn {
                            role,
                            content,
                            tool_calls,
                            depth: 1,
                        });
                    }
                }
            }
            AiDatasetKind::SftShareGpt => {
                blocks.push(PreviewBlock::Section {
                    title: format!("Record #{rec_no}"),
                    badge: Some("ShareGPT".into()),
                    depth: 0,
                });
                if let Some(convs) = rec.get("conversations").and_then(Value::as_array) {
                    for c in convs {
                        let role = c
                            .get("from")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let content = extract_text(c.get("value"));
                        blocks.push(PreviewBlock::ChatTurn {
                            role,
                            content,
                            tool_calls: None,
                            depth: 1,
                        });
                    }
                }
            }
            AiDatasetKind::SftAlpaca => {
                blocks.push(PreviewBlock::Section {
                    title: format!("Record #{rec_no}"),
                    badge: Some("Alpaca".into()),
                    depth: 0,
                });
                let instr = extract_text(rec.get("instruction"));
                let inp = extract_text(rec.get("input"));
                let user_msg = if inp.trim().is_empty() {
                    instr
                } else {
                    format!("{instr}\n\n[Input]\n{inp}")
                };
                blocks.push(PreviewBlock::ChatTurn {
                    role: "user".into(),
                    content: user_msg,
                    tool_calls: None,
                    depth: 1,
                });
                blocks.push(PreviewBlock::ChatTurn {
                    role: "assistant".into(),
                    content: extract_text(rec.get("output")),
                    tool_calls: None,
                    depth: 1,
                });
            }
            AiDatasetKind::SftGemini => {
                blocks.push(PreviewBlock::Section {
                    title: format!("Record #{rec_no}"),
                    badge: Some("Gemini".into()),
                    depth: 0,
                });
                if let Some(sys) = rec.get("systemInstruction") {
                    let sys_text = extract_gemini_parts(sys.get("parts")).trim().to_string();
                    if !sys_text.is_empty() {
                        blocks.push(PreviewBlock::ChatTurn {
                            role: "system".into(),
                            content: sys_text,
                            tool_calls: None,
                            depth: 1,
                        });
                    }
                }
                if let Some(contents) = rec.get("contents").and_then(Value::as_array) {
                    for c in contents {
                        let role = c
                            .get("role")
                            .and_then(Value::as_str)
                            .unwrap_or("user")
                            .to_string();
                        blocks.push(PreviewBlock::ChatTurn {
                            role,
                            content: extract_gemini_parts(c.get("parts")),
                            tool_calls: None,
                            depth: 1,
                        });
                    }
                }
            }
            AiDatasetKind::RlhfDpo => {
                let prompt = extract_text(rec.get("prompt"));
                let chosen = format_preference_side(rec.get("chosen"));
                let rejected = format_preference_side(rec.get("rejected"));
                let mut pair_warnings = Vec::new();
                if chosen.trim() == rejected.trim() && !chosen.trim().is_empty() {
                    pair_warnings.push("chosen == rejected".to_string());
                }
                if chosen.trim().is_empty() || rejected.trim().is_empty() {
                    pair_warnings.push("empty chosen or rejected".to_string());
                }
                blocks.push(PreviewBlock::Section {
                    title: format!("Preference Pair #{rec_no}"),
                    badge: Some("DPO / RLHF".into()),
                    depth: 0,
                });
                blocks.push(PreviewBlock::PreferencePair {
                    prompt,
                    chosen,
                    rejected,
                    warnings: pair_warnings,
                });
            }
            AiDatasetKind::RlhfKto => {
                let lbl = rec.get("label").and_then(Value::as_bool);
                let badge = match lbl {
                    Some(true) => "KTO · Desirable (true)",
                    Some(false) => "KTO · Undesirable (false)",
                    None => "KTO",
                };
                blocks.push(PreviewBlock::Section {
                    title: format!("Feedback #{rec_no}"),
                    badge: Some(badge.into()),
                    depth: 0,
                });
                blocks.push(PreviewBlock::ChatTurn {
                    role: "user".into(),
                    content: extract_text(rec.get("prompt")),
                    tool_calls: None,
                    depth: 1,
                });
                blocks.push(PreviewBlock::ChatTurn {
                    role: if lbl == Some(false) { "rejected" } else { "assistant" }.into(),
                    content: extract_text(rec.get("completion").or_else(|| rec.get("response"))),
                    tool_calls: None,
                    depth: 1,
                });
            }
            AiDatasetKind::RlhfReward => {
                let score_str = rec
                    .get("score")
                    .or_else(|| rec.get("reward"))
                    .or_else(|| rec.get("rating"))
                    .map(|v| format!("score: {v}"));
                blocks.push(PreviewBlock::Section {
                    title: format!("Reward / Critique #{rec_no}"),
                    badge: score_str,
                    depth: 0,
                });
                if rec.get("prompt").is_some() {
                    blocks.push(PreviewBlock::ChatTurn {
                        role: "user".into(),
                        content: extract_text(rec.get("prompt")),
                        tool_calls: None,
                        depth: 1,
                    });
                }
                if rec.get("response").is_some() {
                    blocks.push(PreviewBlock::ChatTurn {
                        role: "assistant".into(),
                        content: extract_text(rec.get("response")),
                        tool_calls: None,
                        depth: 1,
                    });
                }
                if let Some(crit) = rec.get("critique").or_else(|| rec.get("rubric")) {
                    blocks.push(PreviewBlock::KeyValue {
                        depth: 1,
                        key: "critique / rubric".into(),
                        path: String::new(),
                        value: extract_text(Some(crit)),
                        kind: ValueKind::String,
                    });
                }
                if let Some(cands) = rec.get("candidates").and_then(Value::as_array) {
                    let rows: Vec<Vec<TableCellValue>> = cands
                        .iter()
                        .enumerate()
                        .map(|(ci, c)| {
                            vec![
                                TableCellValue {
                                    text: format!("#{}", ci + 1),
                                    kind: ValueKind::Number,
                                },
                                TableCellValue {
                                    text: extract_text(Some(c)),
                                    kind: ValueKind::String,
                                },
                            ]
                        })
                        .collect();
                    blocks.push(PreviewBlock::Table {
                        title: Some("Ranked Candidates".into()),
                        depth: 1,
                        headers: vec!["Rank".into(), "Candidate".into()],
                        rows,
                    });
                }
            }
        }
    }

    let summary = super::visual::summarize_blocks(
        byte_size,
        field_count,
        &blocks,
        Some(primary_kind.label().to_string()),
        warnings,
    );
    (summary, blocks)
}

fn extract_text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn extract_gemini_parts(v: Option<&Value>) -> String {
    let Some(Value::Array(parts)) = v else {
        return extract_text(v);
    };
    parts
        .iter()
        .filter_map(|p| {
            p.get("text")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| Some(p.to_string()))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_preference_side(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|m| {
                let role = m
                    .get("role")
                    .or_else(|| m.get("from"))
                    .and_then(Value::as_str)
                    .unwrap_or("turn");
                let text = extract_text(m.get("content").or_else(|| m.get("value")));
                format!("[{role}] {text}")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
    }
}

// Keep unused-import-free even if `push` isn't directly called outside json_fields.
#[allow(dead_code)]
fn _keep_push_link() {
    let _ = push;
}
