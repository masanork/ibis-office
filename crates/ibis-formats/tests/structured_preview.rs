// SPDX-License-Identifier: AGPL-3.0-only

use ibis_doc_model::sd_jwt::base64url_decode;
use ibis_formats::structured_preview;

const JSON: &[u8] = include_bytes!("../../../test/fixtures/structured-preview/training.json");
const XML: &[u8] = include_bytes!("../../../test/fixtures/structured-preview/training.xml");
const SD_JWT: &[u8] = include_bytes!("../../../test/fixtures/structured-preview/training.sd-jwt");
const MDOC: &[u8] = include_bytes!("../../../test/fixtures/structured-preview/training.mdoc");

#[test]
fn training_record_fields_are_visible_and_undisclosed_values_are_absent() {
    let binary = base64url_decode(std::str::from_utf8(MDOC).unwrap().trim()).unwrap();
    for (extension, data) in [
        ("json", JSON),
        ("xml", XML),
        ("sd-jwt", SD_JWT),
        ("mdoc", MDOC),
        ("mdoc", binary.as_slice()),
    ] {
        let preview = structured_preview::read(extension, data).unwrap();
        assert!(
            preview
                .fields
                .iter()
                .any(|field| field.name == "研修名" && field.value.contains("安全研修"))
        );
        assert!(
            preview
                .fields
                .iter()
                .any(|field| field.name == "修了日" && field.value.contains("2026-04-01"))
        );
        if let Some(credential) = preview.credential {
            assert_eq!(credential.matching_digests, 2);
            assert!(
                !preview
                    .fields
                    .iter()
                    .any(|field| field.name == "管理番号" || field.value.contains("00123"))
            );
        } else {
            let field = preview
                .fields
                .iter()
                .find(|field| field.name == "管理番号")
                .unwrap();
            assert_eq!(
                field.value,
                if extension == "xml" {
                    "00123"
                } else {
                    "\"00123\""
                }
            );
        }
    }
}

#[test]
fn nested_values_keep_distinct_paths_and_xml_does_not_resolve_entities() {
    let json = structured_preview::read(
        "JSON",
        br#"{"a/b":[1, false, null],"a":{"b":2},"big":1234567890123456789012345}"#,
    )
    .unwrap();
    let fields: Vec<_> = json
        .fields
        .iter()
        .map(|field| (field.path.as_str(), field.value.as_str()))
        .collect();
    assert!(fields.contains(&("/a~1b/1", "false")));
    assert!(fields.contains(&("/a/b", "2")));
    assert!(fields.contains(&("/big", "1234567890123456789012345")));

    let data = br#"<r xmlns:p="urn:sample"><p:item code="001">x</p:item><p:item>y</p:item></r>"#;
    let xml = structured_preview::read("xml", data).unwrap();
    let paths: Vec<_> = xml.fields.iter().map(|field| field.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/r/{urn:sample}item[1]/@code",
            "/r/{urn:sample}item[1]/#text[1]",
            "/r/{urn:sample}item[2]/#text[1]"
        ]
    );
    assert!(
        structured_preview::read(
            "xml",
            br#"<!DOCTYPE r [<!ENTITY external SYSTEM "file:///unavailable">]><r>&external;</r>"#
        )
        .is_err()
    );
    assert!(structured_preview::read("json", b"{not-json").is_err());
}

#[test]
fn builds_visual_hierarchy_and_promotes_uniform_arrays_and_xml_elements_to_tables() {
    use structured_preview::{PreviewBlock, ValueKind};

    let json = structured_preview::read(
        "json",
        br#"{
            "service": "auth-api",
            "replicas": 3,
            "enabled": true,
            "endpoints": [
                {"method": "POST", "path": "/login", "status": 200},
                {"method": "GET", "path": "/me", "status": 200}
            ]
        }"#,
    )
    .unwrap();
    assert_eq!(json.summary.table_count, 1);
    assert!(json.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::KeyValue { key, kind: ValueKind::Boolean, .. } if key == "enabled"
    )));
    assert!(json.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Table { headers, rows, .. }
            if headers == &["method", "path", "status"] && rows.len() == 2
    )));

    let yaml = structured_preview::read(
        "yaml",
        b"name: staging\nports:\n  - container: 8080\n    protocol: TCP\n  - container: 8443\n    protocol: TCP\n",
    )
    .unwrap();
    assert_eq!(yaml.format, "YAML");
    assert_eq!(yaml.summary.table_count, 1);

    let xml = structured_preview::read(
        "xml",
        br#"<catalog><book id="1"><title>Rust</title><price>3200</price></book><book id="2"><title>Go</title><price>2800</price></book></catalog>"#,
    )
    .unwrap();
    assert_eq!(xml.summary.table_count, 1);
    assert!(xml.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Table { headers, rows, .. }
            if headers == &["@id", "title", "price"] && rows.len() == 2
    )));
}

#[test]
fn detects_and_validates_ai_sft_and_rlhf_datasets_across_json_and_jsonl() {
    use structured_preview::{CalloutTone, PreviewBlock};

    let sft_jsonl = concat!(
        r#"{"messages":[{"role":"system","content":"You are helpful."},{"role":"user","content":"Hi"},{"role":"assistant","content":"Hello!"}]}"#,
        "\n",
        r#"{"messages":[{"role":"user","content":"Empty?"},{"role":"assistant","content":""}]}"#,
        "\n",
        "{broken json line\n"
    );
    let sft = structured_preview::read("jsonl", sft_jsonl.as_bytes()).unwrap();
    assert_eq!(sft.format, "JSONL");
    assert_eq!(
        sft.summary.detected_schema.as_deref(),
        Some("AI SFT (OpenAI / ChatML)")
    );
    assert_eq!(sft.summary.warnings.len(), 2);
    assert!(sft.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Callout { tone: CalloutTone::Danger, .. }
    )));
    assert!(sft.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::ChatTurn { role, content, .. } if role == "assistant" && content == "Hello!"
    )));

    let dpo_jsonl = concat!(
        r#"{"prompt":"Explain Rust","chosen":"Rust ensures memory safety.","rejected":"I don't know."}"#,
        "\n",
        r#"{"prompt":"Dup","chosen":"Same","rejected":"Same"}"#,
        "\n"
    );
    let dpo = structured_preview::read("ndjson", dpo_jsonl.as_bytes()).unwrap();
    assert_eq!(
        dpo.summary.detected_schema.as_deref(),
        Some("AI RLHF / DPO Preference Pairs")
    );
    assert!(dpo
        .summary
        .warnings
        .iter()
        .any(|w| w.contains("identical chosen and rejected")));
    assert!(dpo.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::PreferencePair { prompt, chosen, rejected, .. }
            if prompt == "Explain Rust" && chosen.contains("memory safety") && rejected == "I don't know."
    )));

    let kto = structured_preview::read(
        "jsonl",
        br#"{"prompt":"Q1","completion":"Good","label":true}
{"prompt":"Q2","completion":"Bad","label":false}"#,
    )
    .unwrap();
    assert_eq!(
        kto.summary.detected_schema.as_deref(),
        Some("AI RLHF / KTO Binary Feedback")
    );
    assert!(kto.summary.warnings.is_empty());

    let alpaca = structured_preview::read(
        "json",
        r#"{"instruction":"Translate","input":"Hello","output":"こんにちは"}"#.as_bytes(),
    )
    .unwrap();
    assert_eq!(
        alpaca.summary.detected_schema.as_deref(),
        Some("AI SFT (Alpaca / Instruction)")
    );
}
