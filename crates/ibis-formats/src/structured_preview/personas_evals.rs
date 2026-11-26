// SPDX-License-Identifier: AGPL-3.0-only
//! Dedicated visual layouts for AI Personas, Character Cards (v2/v3),
//! Prompt Specs (`.prompt.yml`), Eval results, and Agent Trajectories.

use serde_json::Value;

use super::visual::{
    PreviewBlock, PreviewSummary, TableCellValue, ValueKind, summarize_blocks,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PersonaEvalKind {
    CharacterCard,
    PersonaSpec,
    PromptSpec,
    EvalResult,
    AgentTrajectory,
}

impl PersonaEvalKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            PersonaEvalKind::CharacterCard => "AI Character Card (v2/v3)",
            PersonaEvalKind::PersonaSpec => "AI Persona / System Prompt",
            PersonaEvalKind::PromptSpec => "AI Prompt Spec (.prompt.yml)",
            PersonaEvalKind::EvalResult => "AI Evaluation Results",
            PersonaEvalKind::AgentTrajectory => "AI Agent Trajectory",
        }
    }
}

pub(super) fn detect_persona_eval_kind(val: &Value) -> Option<PersonaEvalKind> {
    let Value::Object(map) = val else {
        return None;
    };
    if map
        .get("spec")
        .and_then(Value::as_str)
        .is_some_and(|s| s.starts_with("chara_card_v"))
        && map.get("data").is_some_and(Value::is_object)
    {
        return Some(PersonaEvalKind::CharacterCard);
    }
    if map.contains_key("model")
        && (map.get("messages").is_some_and(Value::is_array)
            || map.contains_key("modelParameters")
            || map.contains_key("testData"))
    {
        return Some(PersonaEvalKind::PromptSpec);
    }
    if (map.contains_key("expected") || map.contains_key("actual") || map.contains_key("pass") || map.contains_key("passed"))
        && (map.contains_key("input") || map.contains_key("prompt") || map.contains_key("score"))
    {
        return Some(PersonaEvalKind::EvalResult);
    }
    if map.get("results").is_some_and(|r| {
        r.as_array()
            .is_some_and(|arr| arr.first().is_some_and(|f| f.get("pass").is_some() || f.get("passed").is_some() || f.get("expected").is_some()))
    }) {
        return Some(PersonaEvalKind::EvalResult);
    }
    if map
        .get("steps")
        .or_else(|| map.get("trajectory"))
        .and_then(Value::as_array)
        .is_some_and(|arr| {
            arr.first().is_some_and(|s| {
                s.get("thought").is_some()
                    || s.get("action").is_some()
                    || s.get("observation").is_some()
                    || s.get("tool_calls").is_some()
            })
        })
    {
        return Some(PersonaEvalKind::AgentTrajectory);
    }
    if (map.contains_key("name") || map.contains_key("role"))
        && (map.contains_key("personality")
            || map.contains_key("traits")
            || map.contains_key("tone")
            || map.contains_key("constraints")
            || map.contains_key("system_prompt"))
    {
        return Some(PersonaEvalKind::PersonaSpec);
    }
    None
}

pub(super) fn try_build_persona_eval_preview(
    records: &[&Value],
    byte_size: usize,
    field_count: usize,
) -> Option<(PreviewSummary, Vec<PreviewBlock>)> {
    let kind = records.iter().find_map(|r| detect_persona_eval_kind(r))?;
    let mut blocks = Vec::new();
    match kind {
        PersonaEvalKind::CharacterCard => build_character_card(records[0], &mut blocks),
        PersonaEvalKind::PersonaSpec => build_persona_spec(records[0], &mut blocks),
        PersonaEvalKind::PromptSpec => build_prompt_spec(records[0], &mut blocks),
        PersonaEvalKind::EvalResult => build_eval_results(records, &mut blocks),
        PersonaEvalKind::AgentTrajectory => build_trajectories(records, &mut blocks),
    }
    let summary = summarize_blocks(
        byte_size,
        field_count,
        &blocks,
        Some(kind.label().to_string()),
        Vec::new(),
    );
    Some((summary, blocks))
}

fn str_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|x| x.as_str().map(ToString::to_string).or_else(|| Some(x.to_string())))
            .collect::<Vec<_>>()
            .join(", "),
        Some(other) => other.to_string(),
    }
}

fn build_character_card(val: &Value, blocks: &mut Vec<PreviewBlock>) {
    let data = val.get("data").unwrap_or(val);
    let spec = str_of(val.get("spec"));
    let profile_fields = [
        ("Spec", spec),
        ("Name", str_of(data.get("name"))),
        ("Creator", str_of(data.get("creator"))),
        ("Version", str_of(data.get("character_version"))),
        ("Tags", str_of(data.get("tags"))),
    ];
    let rows: Vec<Vec<TableCellValue>> = profile_fields
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            vec![
                TableCellValue {
                    text: k.into(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: v,
                    kind: ValueKind::String,
                },
            ]
        })
        .collect();
    if !rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some("Character Profile".into()),
            depth: 0,
            headers: vec!["Field".into(), "Value".into()],
            rows,
        });
    }
    for (label, key) in [
        ("Description", "description"),
        ("Personality", "personality"),
        ("Scenario", "scenario"),
        ("System Prompt", "system_prompt"),
        ("Post-History Instructions", "post_history_instructions"),
    ] {
        let txt = str_of(data.get(key));
        if !txt.trim().is_empty() {
            blocks.push(PreviewBlock::KeyValue {
                depth: 0,
                key: label.into(),
                path: format!("/data/{key}"),
                value: txt,
                kind: ValueKind::String,
            });
        }
    }
    let first_mes = str_of(data.get("first_mes"));
    if !first_mes.trim().is_empty() {
        blocks.push(PreviewBlock::ChatTurn {
            role: "assistant".into(),
            content: first_mes,
            tool_calls: None,
            depth: 0,
        });
    }
    let mes_example = str_of(data.get("mes_example"));
    if !mes_example.trim().is_empty() {
        blocks.push(PreviewBlock::ChatTurn {
            role: "system".into(),
            content: format!("[Example Dialogue]\n{mes_example}"),
            tool_calls: None,
            depth: 0,
        });
    }
    if let Some(entries) = data
        .get("character_book")
        .and_then(|b| b.get("entries"))
        .and_then(Value::as_array)
        && !entries.is_empty()
    {
        let lore_rows: Vec<Vec<TableCellValue>> = entries
            .iter()
            .map(|e| {
                vec![
                    TableCellValue {
                        text: str_of(e.get("keys")),
                        kind: ValueKind::Attribute,
                    },
                    TableCellValue {
                        text: str_of(e.get("content")),
                        kind: ValueKind::String,
                    },
                    TableCellValue {
                        text: str_of(e.get("enabled")),
                        kind: ValueKind::Boolean,
                    },
                ]
            })
            .collect();
        blocks.push(PreviewBlock::Table {
            title: Some(format!("Lorebook ({} entries)", lore_rows.len())),
            depth: 0,
            headers: vec!["Keys".into(), "Content".into(), "Enabled".into()],
            rows: lore_rows,
        });
    }
}

fn build_persona_spec(val: &Value, blocks: &mut Vec<PreviewBlock>) {
    let profile_fields = [
        ("Name", str_of(val.get("name"))),
        ("Role", str_of(val.get("role"))),
        ("Tone", str_of(val.get("tone"))),
        ("Traits", str_of(val.get("traits").or_else(|| val.get("personality")))),
        ("Constraints", str_of(val.get("constraints"))),
    ];
    let rows: Vec<Vec<TableCellValue>> = profile_fields
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            vec![
                TableCellValue {
                    text: k.into(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: v,
                    kind: ValueKind::String,
                },
            ]
        })
        .collect();
    if !rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some("Persona Definition".into()),
            depth: 0,
            headers: vec!["Attribute".into(), "Detail".into()],
            rows,
        });
    }
    let sys = str_of(val.get("system_prompt").or_else(|| val.get("instructions")));
    if !sys.trim().is_empty() {
        blocks.push(PreviewBlock::ChatTurn {
            role: "system".into(),
            content: sys,
            tool_calls: None,
            depth: 0,
        });
    }
    if let Some(examples) = val.get("examples").and_then(Value::as_array) {
        for (i, ex) in examples.iter().enumerate() {
            blocks.push(PreviewBlock::Section {
                title: format!("Example #{}", i + 1),
                badge: None,
                depth: 0,
            });
            if let Some(u) = ex.get("user").or_else(|| ex.get("input")) {
                blocks.push(PreviewBlock::ChatTurn {
                    role: "user".into(),
                    content: str_of(Some(u)),
                    tool_calls: None,
                    depth: 1,
                });
            }
            if let Some(a) = ex.get("assistant").or_else(|| ex.get("output")) {
                blocks.push(PreviewBlock::ChatTurn {
                    role: "assistant".into(),
                    content: str_of(Some(a)),
                    tool_calls: None,
                    depth: 1,
                });
            }
        }
    }
}

fn build_prompt_spec(val: &Value, blocks: &mut Vec<PreviewBlock>) {
    let mut cfg_rows = vec![vec![
        TableCellValue {
            text: "model".into(),
            kind: ValueKind::Attribute,
        },
        TableCellValue {
            text: str_of(val.get("model")),
            kind: ValueKind::String,
        },
    ]];
    if let Some(Value::Object(params)) = val.get("modelParameters") {
        for (k, v) in params {
            cfg_rows.push(vec![
                TableCellValue {
                    text: k.clone(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: v.to_string(),
                    kind: ValueKind::Number,
                },
            ]);
        }
    }
    blocks.push(PreviewBlock::Table {
        title: Some("Model & Parameters".into()),
        depth: 0,
        headers: vec!["Setting".into(), "Value".into()],
        rows: cfg_rows,
    });
    if let Some(msgs) = val.get("messages").and_then(Value::as_array) {
        blocks.push(PreviewBlock::Section {
            title: "Prompt Messages".into(),
            badge: Some(format!("{} turns", msgs.len())),
            depth: 0,
        });
        for m in msgs {
            blocks.push(PreviewBlock::ChatTurn {
                role: str_of(m.get("role")),
                content: str_of(m.get("content")),
                tool_calls: None,
                depth: 1,
            });
        }
    }
    if let Some(tests) = val.get("testData").and_then(Value::as_array)
        && !tests.is_empty()
    {
        let mut fake_fields = Vec::new();
        let _ = super::visual::build_json_blocks(
            &Value::Array(tests.clone()),
            "/testData",
            "testData",
            0,
            &mut fake_fields,
        );
        blocks.extend(fake_fields);
    }
}

fn build_eval_results(records: &[&Value], blocks: &mut Vec<PreviewBlock>) {
    let items: Vec<&Value> = if records.len() == 1
        && let Some(arr) = records[0].get("results").and_then(Value::as_array)
    {
        arr.iter().collect()
    } else {
        records.to_vec()
    };

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut scores = Vec::new();
    let mut rows = Vec::new();

    for (idx, item) in items.iter().take(50).enumerate() {
        let pass_opt = item
            .get("pass")
            .or_else(|| item.get("passed"))
            .and_then(Value::as_bool);
        match pass_opt {
            Some(true) => passed += 1,
            Some(false) => failed += 1,
            None => {}
        }
        if let Some(s) = item.get("score").and_then(Value::as_f64) {
            scores.push(s);
        }
        let status_cell = match pass_opt {
            Some(true) => TableCellValue {
                text: "PASS".into(),
                kind: ValueKind::String,
            },
            Some(false) => TableCellValue {
                text: "FAIL".into(),
                kind: ValueKind::Null,
            },
            None => TableCellValue {
                text: str_of(item.get("score")),
                kind: ValueKind::Number,
            },
        };
        rows.push(vec![
            TableCellValue {
                text: format!("#{}", idx + 1),
                kind: ValueKind::Number,
            },
            status_cell,
            TableCellValue {
                text: str_of(item.get("input").or_else(|| item.get("prompt"))),
                kind: ValueKind::String,
            },
            TableCellValue {
                text: str_of(item.get("expected")),
                kind: ValueKind::String,
            },
            TableCellValue {
                text: str_of(item.get("actual").or_else(|| item.get("output"))),
                kind: ValueKind::String,
            },
        ]);
    }

    let total = passed + failed;
    let mut stat_rows = vec![vec![
        TableCellValue {
            text: "Cases".into(),
            kind: ValueKind::Attribute,
        },
        TableCellValue {
            text: items.len().to_string(),
            kind: ValueKind::Number,
        },
    ]];
    if total > 0 {
        let rate = (passed as f64 / total as f64) * 100.0;
        stat_rows.push(vec![
            TableCellValue {
                text: "Pass Rate".into(),
                kind: ValueKind::Attribute,
            },
            TableCellValue {
                text: format!("{passed}/{total} ({rate:.1}%)"),
                kind: ValueKind::Boolean,
            },
        ]);
    }
    if !scores.is_empty() {
        let avg = scores.iter().sum::<f64>() / scores.len() as f64;
        stat_rows.push(vec![
            TableCellValue {
                text: "Avg Score".into(),
                kind: ValueKind::Attribute,
            },
            TableCellValue {
                text: format!("{avg:.3}"),
                kind: ValueKind::Number,
            },
        ]);
    }
    blocks.push(PreviewBlock::Table {
        title: Some("Evaluation Summary".into()),
        depth: 0,
        headers: vec!["Metric".into(), "Value".into()],
        rows: stat_rows,
    });
    blocks.push(PreviewBlock::Table {
        title: Some("Evaluation Cases".into()),
        depth: 0,
        headers: vec![
            "#".into(),
            "Status".into(),
            "Input".into(),
            "Expected".into(),
            "Actual".into(),
        ],
        rows,
    });
}

fn build_trajectories(records: &[&Value], blocks: &mut Vec<PreviewBlock>) {
    for (ri, rec) in records.iter().take(10).enumerate() {
        let steps = rec
            .get("steps")
            .or_else(|| rec.get("trajectory"))
            .and_then(Value::as_array);
        let Some(steps) = steps else { continue };
        blocks.push(PreviewBlock::Section {
            title: format!("Trajectory #{}", ri + 1),
            badge: Some(format!("{} steps", steps.len())),
            depth: 0,
        });
        for (si, step) in steps.iter().enumerate() {
            let thought = str_of(step.get("thought"));
            let action = str_of(step.get("action").or_else(|| step.get("tool_calls")));
            let obs = str_of(step.get("observation"));
            if !thought.is_empty() {
                blocks.push(PreviewBlock::ChatTurn {
                    role: format!("step {} · thought", si + 1),
                    content: thought,
                    tool_calls: None,
                    depth: 1,
                });
            }
            if !action.is_empty() {
                blocks.push(PreviewBlock::ChatTurn {
                    role: "tool".into(),
                    content: action,
                    tool_calls: None,
                    depth: 1,
                });
            }
            if !obs.is_empty() {
                blocks.push(PreviewBlock::ChatTurn {
                    role: "assistant".into(),
                    content: obs,
                    tool_calls: None,
                    depth: 1,
                });
            }
        }
    }
}
