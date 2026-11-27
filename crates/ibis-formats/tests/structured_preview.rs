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
        PreviewBlock::Table { title: Some(t), rows, .. }
            if t.starts_with("Record Index") && rows.len() == 3
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

#[test]
fn detects_character_cards_personas_prompt_specs_evals_and_trajectories() {
    use structured_preview::PreviewBlock;

    let card = structured_preview::read(
        "json",
        r#"{
            "spec": "chara_card_v2",
            "data": {
                "name": "Aoi",
                "personality": "Calm and analytical",
                "first_mes": "こんにちは、仕様を確認しましょう。",
                "character_book": {
                    "entries": [{"keys": ["API"], "content": "REST v2", "enabled": true}]
                }
            }
        }"#
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        card.summary.detected_schema.as_deref(),
        Some("AI Character Card (v2/v3)")
    );
    assert!(card.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Table { title: Some(t), .. } if t.contains("Lorebook")
    )));

    let prompt_yml = structured_preview::read(
        "yml",
        b"model: gpt-4o\nmodelParameters:\n  temperature: 0.2\nmessages:\n  - role: system\n    content: Be concise.\n  - role: user\n    content: Summarize.\n",
    )
    .unwrap();
    assert_eq!(
        prompt_yml.summary.detected_schema.as_deref(),
        Some("AI Prompt Spec (.prompt.yml)")
    );

    let eval_jsonl = concat!(
        r#"{"input":"2+2","expected":"4","actual":"4","pass":true,"score":1.0}"#,
        "\n",
        r#"{"input":"3+3","expected":"6","actual":"7","pass":false,"score":0.0}"#,
        "\n"
    );
    let eval = structured_preview::read("jsonl", eval_jsonl.as_bytes()).unwrap();
    assert_eq!(
        eval.summary.detected_schema.as_deref(),
        Some("AI Evaluation Results")
    );
    assert!(eval.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Table { title: Some(t), rows, .. } if t == "Evaluation Cases" && rows.len() == 2
    )));

    let traj = structured_preview::read(
        "json",
        br#"{"steps":[{"thought":"Need schema","action":"read_file","observation":"OK"}]}"#,
    )
    .unwrap();
    assert_eq!(
        traj.summary.detected_schema.as_deref(),
        Some("AI Agent Trajectory")
    );
    assert!(traj.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::ChatTurn { role, content, .. } if role == "tool" && content == "read_file"
    )));
}

#[test]
fn reads_jupyter_notebook_toml_json_schema_and_openapi() {
    use structured_preview::{CalloutTone, PreviewBlock};

    let ipynb = r##"{
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {"kernelspec": {"display_name": "Python 3", "language": "python"}},
        "cells": [
            {"cell_type": "markdown", "source": ["# Analysis\n", "Summary"]},
            {"cell_type": "code", "execution_count": 1, "source": ["print(1+1)"], "outputs": [
                {"output_type": "stream", "name": "stdout", "text": ["2\n"]}
            ]},
            {"cell_type": "code", "execution_count": 2, "source": ["1/0"], "outputs": [
                {"output_type": "error", "ename": "ZeroDivisionError", "evalue": "division by zero"}
            ]}
        ]
    }"##;
    let nb = structured_preview::read("ipynb", ipynb.as_bytes()).unwrap();
    assert_eq!(nb.format, "Jupyter Notebook");
    assert!(nb.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Callout { tone: CalloutTone::Danger, title, .. } if title == "ZeroDivisionError"
    )));

    let toml_src = b"[project]\nname = \"eval-pipeline\"\nversion = \"0.2.0\"\n\n[[models]]\nid = \"gpt-4o\"\ntemp = 0.2\n\n[[models]]\nid = \"claude\"\ntemp = 0.0\n";
    let toml_prev = structured_preview::read("toml", toml_src).unwrap();
    assert_eq!(toml_prev.format, "TOML");
    assert_eq!(toml_prev.summary.table_count, 1);

    let schema = br#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "UserProfile",
        "type": "object",
        "required": ["id"],
        "properties": {
            "id": {"type": "string", "format": "uuid", "description": "User ID"},
            "age": {"type": "integer", "default": 20}
        }
    }"#;
    let sch = structured_preview::read("json", schema).unwrap();
    assert_eq!(sch.summary.detected_schema.as_deref(), Some("JSON Schema"));
    assert!(sch.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Table { title: Some(t), rows, .. } if t.starts_with("Properties") && rows.len() == 2
    )));

    let openapi = b"openapi: '3.1.0'\ninfo:\n  title: Items API\n  version: '1.0'\npaths:\n  /items:\n    get:\n      summary: List items\n      responses:\n        '200':\n          description: OK\n";
    let oa = structured_preview::read("yaml", openapi).unwrap();
    assert_eq!(oa.summary.detected_schema.as_deref(), Some("OpenAPI (Items API)"));
}

#[test]
fn reads_ontology_jsonld_rdf_owl_and_turtle() {
    use structured_preview::PreviewBlock;

    let jsonld = br#"{
        "@context": {
            "schema": "https://schema.org/",
            "rdfs": "http://www.w3.org/2000/01/rdf-schema#"
        },
        "@graph": [
            {"@id": "ex:Person", "@type": "rdfs:Class", "rdfs:label": "Person"},
            {"@id": "ex:alice", "@type": "ex:Person", "schema:name": "Alice"}
        ]
    }"#;
    let ld = structured_preview::read("jsonld", jsonld).unwrap();
    assert_eq!(ld.format, "JSON-LD");
    assert_eq!(ld.summary.detected_schema.as_deref(), Some("JSON-LD"));
    assert!(ld.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Table { title: Some(t), rows, .. } if t.starts_with("Graph Entities") && rows.len() == 2
    )));

    let owl = br#"<?xml version="1.0"?>
    <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
             xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#"
             xmlns:owl="http://www.w3.org/2002/07/owl#">
      <owl:Ontology rdf:about="http://example.org/onto">
        <rdfs:label>Example Ontology</rdfs:label>
      </owl:Ontology>
      <owl:Class rdf:about="http://example.org/onto#Animal">
        <rdfs:label>Animal</rdfs:label>
      </owl:Class>
      <owl:Class rdf:about="http://example.org/onto#Dog">
        <rdfs:subClassOf rdf:resource="http://example.org/onto#Animal"/>
        <rdfs:label>Dog</rdfs:label>
      </owl:Class>
      <owl:ObjectProperty rdf:about="http://example.org/onto#owns">
        <rdfs:domain rdf:resource="http://example.org/onto#Animal"/>
        <rdfs:label>owns</rdfs:label>
      </owl:ObjectProperty>
    </rdf:RDF>"#;
    let owl_prev = structured_preview::read("owl", owl).unwrap();
    assert_eq!(owl_prev.format, "OWL");
    assert_eq!(owl_prev.summary.detected_schema.as_deref(), Some("OWL / RDF-XML"));
    assert!(owl_prev.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Table { title: Some(t), rows, .. } if t.starts_with("Classes & Hierarchy") && rows.len() == 2
    )));

    let ttl = br#"
        @prefix ex: <http://example.org/> .
        @prefix owl: <http://www.w3.org/2002/07/owl#> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

        ex:Agent a owl:Class ;
            rdfs:label "Agent" .

        ex:Alice a ex:Agent ;
            rdfs:label "Alice"@en ;
            ex:age 30 .
    "#;
    let ttl_prev = structured_preview::read("ttl", ttl).unwrap();
    assert_eq!(ttl_prev.format, "Turtle");
    assert_eq!(ttl_prev.summary.detected_schema.as_deref(), Some("Turtle"));
    assert!(ttl_prev.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Table { title: Some(t), rows, .. } if t.starts_with("Classes & Hierarchy") && rows.len() == 1
    )));
    assert!(ttl_prev.blocks.iter().any(|b| matches!(
        b,
        PreviewBlock::Section { title, .. } if title == "ex:Alice"
    )));

    let nt = br#"<http://example.org/s> <http://example.org/p> "hello" ."#;
    let nt_prev = structured_preview::read("nt", nt).unwrap();
    assert_eq!(nt_prev.format, "N-Triples");
    assert!(!nt_prev.fields.is_empty());
}

