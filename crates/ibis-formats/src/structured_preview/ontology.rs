// SPDX-License-Identifier: AGPL-3.0-only
//! Ontology and Knowledge Graph preview builders (JSON-LD, RDF/XML, OWL, Turtle, N-Triples, N-Quads, TriG).

use std::collections::{BTreeMap, BTreeSet};

use roxmltree::{Document, Node};
use serde_json::Value;

use super::visual::{
    self, CalloutTone, PreviewBlock, PreviewSummary, TableCellValue, ValueKind,
};
use super::{Field, MAX_FIELDS, json_fields, parse_xml, xml_fields, xml_name};

const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS_NS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL_NS: &str = "http://www.w3.org/2002/07/owl#";
const SKOS_NS: &str = "http://www.w3.org/2004/02/skos/core#";

fn cell(text: impl Into<String>, kind: ValueKind) -> TableCellValue {
    TableCellValue {
        text: text.into(),
        kind,
    }
}

fn push_field(fields: &mut Vec<Field>, name: &str, path: &str, value: &str) {
    if fields.len() < MAX_FIELDS {
        fields.push(Field {
            name: name.to_string(),
            path: path.to_string(),
            value: value.to_string(),
        });
    }
}

fn compact_iri(iri: &str, prefixes: &[(String, String)]) -> String {
    for (pfx, base) in prefixes {
        if !base.is_empty() && iri.starts_with(base.as_str()) {
            let local = &iri[base.len()..];
            if pfx.is_empty() {
                return format!(":{local}");
            }
            return format!("{pfx}:{local}");
        }
    }
    if let Some((_, tail)) = iri.rsplit_once('#') {
        if !tail.is_empty() {
            return tail.to_string();
        }
    }
    if let Some((_, tail)) = iri.rsplit_once('/') {
        if !tail.is_empty() {
            return tail.to_string();
        }
    }
    iri.to_string()
}

// ============================================================================
// JSON-LD (.jsonld and auto-detected .json)
// ============================================================================

pub fn read_jsonld(
    bytes: &[u8],
    raw_len: usize,
) -> Result<(Vec<Field>, PreviewSummary, Vec<PreviewBlock>), String> {
    let data: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid JSON-LD")?;
    let mut fields = Vec::new();
    json_fields(&data, "", "/", 0, &mut fields)?;
    if let Some((summary, blocks)) = try_build_jsonld_blocks(&data, raw_len, &fields) {
        return Ok((fields, summary, blocks));
    }
    let mut blocks = Vec::new();
    visual::build_json_blocks(&data, "", "", 0, &mut blocks)?;
    let summary = visual::summarize_blocks(
        raw_len,
        fields.len(),
        &blocks,
        Some("JSON-LD".into()),
        Vec::new(),
    );
    Ok((fields, summary, blocks))
}

pub fn try_build_jsonld_blocks(
    data: &Value,
    raw_len: usize,
    fields: &[Field],
) -> Option<(PreviewSummary, Vec<PreviewBlock>)> {
    let obj = data.as_object()?;
    let has_context = obj.contains_key("@context");
    let has_graph = obj.contains_key("@graph");
    let has_id_or_type = obj.contains_key("@id") || obj.contains_key("@type");
    if !has_context && !has_graph {
        return None;
    }
    if !has_context && !has_id_or_type && !has_graph {
        return None;
    }

    let mut blocks = Vec::new();
    let mut context_rows: Vec<Vec<TableCellValue>> = Vec::new();
    if let Some(ctx) = obj.get("@context") {
        collect_jsonld_context(ctx, &mut context_rows);
    }

    if !context_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some("JSON-LD @context".into()),
            depth: 0,
            headers: vec!["Term / Prefix".into(), "IRI / Definition".into()],
            rows: context_rows,
        });
    }

    let entities: Vec<&Value> = if let Some(graph) = obj.get("@graph").and_then(Value::as_array) {
        graph.iter().collect()
    } else if has_id_or_type {
        vec![data]
    } else {
        Vec::new()
    };

    if !entities.is_empty() {
        let mut entity_rows = Vec::new();
        for (idx, ent) in entities.iter().take(100).enumerate() {
            if let Some(map) = ent.as_object() {
                let id = map
                    .get("@id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("_:b{idx}"));
                let ty = format_jsonld_type(map.get("@type"));
                let label = extract_jsonld_label(map);
                let prop_count = map
                    .keys()
                    .filter(|k| !k.starts_with('@'))
                    .count();
                entity_rows.push(vec![
                    cell(id, ValueKind::String),
                    cell(ty, ValueKind::Attribute),
                    cell(label, ValueKind::String),
                    cell(prop_count.to_string(), ValueKind::Number),
                ]);
            }
        }
        if !entity_rows.is_empty() {
            blocks.push(PreviewBlock::Table {
                title: Some(format!("Graph Entities ({})", entities.len())),
                depth: 0,
                headers: vec![
                    "@id".into(),
                    "@type".into(),
                    "Label / Name".into(),
                    "Properties".into(),
                ],
                rows: entity_rows,
            });
        }

        for (idx, ent) in entities.iter().take(25).enumerate() {
            if let Some(map) = ent.as_object() {
                let id = map
                    .get("@id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Entity #{}", idx + 1));
                let ty = format_jsonld_type(map.get("@type"));
                blocks.push(PreviewBlock::Section {
                    title: id,
                    badge: if ty.is_empty() { None } else { Some(ty) },
                    depth: 1,
                });
                let _ = visual::build_json_blocks(ent, "", "", 2, &mut blocks);
            }
        }
        if entities.len() > 25 {
            blocks.push(PreviewBlock::Callout {
                tone: CalloutTone::Info,
                title: "Graph truncated".into(),
                body: format!(
                    "Showing detailed properties for the first 25 of {} entities.",
                    entities.len()
                ),
            });
        }
    } else {
        let _ = visual::build_json_blocks(data, "", "", 0, &mut blocks);
    }

    let summary = visual::summarize_blocks(
        raw_len,
        fields.len(),
        &blocks,
        Some("JSON-LD".into()),
        Vec::new(),
    );
    Some((summary, blocks))
}

fn collect_jsonld_context(ctx: &Value, rows: &mut Vec<Vec<TableCellValue>>) {
    match ctx {
        Value::String(s) => {
            rows.push(vec![
                cell("@vocab / remote", ValueKind::Attribute),
                cell(s, ValueKind::String),
            ]);
        }
        Value::Object(map) => {
            for (k, v) in map {
                let val_str = match v {
                    Value::String(s) => s.clone(),
                    _ => v.to_string(),
                };
                rows.push(vec![
                    cell(k, ValueKind::Attribute),
                    cell(val_str, ValueKind::String),
                ]);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_jsonld_context(item, rows);
            }
        }
        _ => {}
    }
}

fn format_jsonld_type(val: Option<&Value>) -> String {
    match val {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn extract_jsonld_label(map: &serde_json::Map<String, Value>) -> String {
    for key in [
        "rdfs:label",
        "skos:prefLabel",
        "schema:name",
        "name",
        "title",
        "label",
    ] {
        if let Some(v) = map.get(key) {
            match v {
                Value::String(s) => return s.clone(),
                Value::Object(o) => {
                    if let Some(s) = o.get("@value").and_then(Value::as_str) {
                        return s.to_string();
                    }
                }
                Value::Array(arr) => {
                    if let Some(first) = arr.first() {
                        if let Some(s) = first.as_str() {
                            return s.to_string();
                        }
                        if let Some(s) = first.get("@value").and_then(Value::as_str) {
                            return s.to_string();
                        }
                    }
                }
                _ => {}
            }
        }
    }
    String::new()
}

// ============================================================================
// RDF/XML & OWL (.rdf, .owl, and auto-detected .xml)
// ============================================================================

pub fn read_rdf_xml(
    bytes: &[u8],
    raw_len: usize,
) -> Result<(Vec<Field>, PreviewSummary, Vec<PreviewBlock>), String> {
    let doc = parse_xml(bytes)?;
    let root = doc.root_element();
    let root_path = format!(
        "/{}",
        xml_name(root.tag_name().namespace(), root.tag_name().name())
    );
    let mut fields = Vec::new();
    xml_fields(root, &root_path, 0, &mut fields)?;

    if let Some((summary, blocks)) = try_build_rdf_xml_blocks(&doc, raw_len, &fields) {
        return Ok((fields, summary, blocks));
    }

    let mut blocks = Vec::new();
    visual::build_xml_blocks(root, &root_path, 0, &mut blocks)?;
    let summary = visual::summarize_blocks(raw_len, fields.len(), &blocks, None, Vec::new());
    Ok((fields, summary, blocks))
}

pub fn try_build_rdf_xml_blocks(
    doc: &Document<'_>,
    raw_len: usize,
    fields: &[Field],
) -> Option<(PreviewSummary, Vec<PreviewBlock>)> {
    let root = doc.root_element();
    let root_ns = root.tag_name().namespace().unwrap_or("");
    let root_name = root.tag_name().name();
    let is_rdf = (root_ns == RDF_NS && root_name == "RDF")
        || root_name == "RDF"
        || (root_ns == OWL_NS && root_name == "Ontology");
    if !is_rdf {
        return None;
    }

    let mut prefixes: Vec<(String, String)> = root
        .namespaces()
        .map(|ns| {
            (
                ns.name().unwrap_or("").to_string(),
                ns.uri().to_string(),
            )
        })
        .collect();
    prefixes.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut blocks = Vec::new();
    if !prefixes.is_empty() {
        let ns_rows = prefixes
            .iter()
            .map(|(p, u)| {
                vec![
                    cell(if p.is_empty() { "(default)" } else { p }, ValueKind::Attribute),
                    cell(u, ValueKind::String),
                ]
            })
            .collect();
        blocks.push(PreviewBlock::Table {
            title: Some("Namespaces / Prefixes".into()),
            depth: 0,
            headers: vec!["Prefix".into(), "Namespace IRI".into()],
            rows: ns_rows,
        });
    }

    let mut ontology_rows = Vec::new();
    let mut class_rows = Vec::new();
    let mut prop_rows = Vec::new();
    let mut concept_rows = Vec::new();
    let mut resource_rows = Vec::new();
    let mut has_owl = false;
    let mut has_skos = false;

    for child in root.children().filter(|n| n.is_element()) {
        let c_ns = child.tag_name().namespace().unwrap_or("");
        let c_name = child.tag_name().name();
        let about = attr_rdf_about(child).unwrap_or_default();
        let label = child_text_first(child, &["label", "prefLabel", "title", "name"])
            .unwrap_or_default();
        let comment = child_text_first(child, &["comment", "definition", "description"])
            .unwrap_or_default();
        let summary_desc = if !label.is_empty() && !comment.is_empty() {
            format!("{label} — {comment}")
        } else if !label.is_empty() {
            label.clone()
        } else {
            comment.clone()
        };

        let explicit_types = collect_rdf_types(child);
        let tag_type = if !c_ns.is_empty() {
            format!("{c_ns}{c_name}")
        } else {
            c_name.to_string()
        };

        let is_ontology = (c_ns == OWL_NS && c_name == "Ontology")
            || explicit_types.iter().any(|t| t.ends_with("Ontology"));
        let is_class = (c_ns == OWL_NS && c_name == "Class")
            || (c_ns == RDFS_NS && c_name == "Class")
            || explicit_types
                .iter()
                .any(|t| t == &format!("{OWL_NS}Class") || t == &format!("{RDFS_NS}Class"));
        let is_prop = (c_ns == OWL_NS
            && matches!(
                c_name,
                "ObjectProperty" | "DatatypeProperty" | "AnnotationProperty" | "FunctionalProperty"
            ))
            || (c_ns == RDF_NS && c_name == "Property")
            || explicit_types
                .iter()
                .any(|t| t.ends_with("Property"));
        let is_concept = (c_ns == SKOS_NS && matches!(c_name, "Concept" | "ConceptScheme"))
            || explicit_types.iter().any(|t| t.starts_with(SKOS_NS));

        if c_ns == OWL_NS || explicit_types.iter().any(|t| t.starts_with(OWL_NS)) {
            has_owl = true;
        }
        if c_ns == SKOS_NS || explicit_types.iter().any(|t| t.starts_with(SKOS_NS)) {
            has_skos = true;
        }

        if is_ontology {
            has_owl = true;
            let version = child_text_or_resource(child, "versionInfo").unwrap_or_default();
            ontology_rows.push(vec![
                cell(compact_iri(&about, &prefixes), ValueKind::Attribute),
                cell(label, ValueKind::String),
                cell(version, ValueKind::String),
                cell(comment, ValueKind::String),
            ]);
        } else if is_class {
            let sub_of = collect_child_resources(child, "subClassOf")
                .into_iter()
                .map(|s| compact_iri(&s, &prefixes))
                .collect::<Vec<_>>()
                .join(", ");
            class_rows.push(vec![
                cell(compact_iri(&about, &prefixes), ValueKind::Attribute),
                cell(sub_of, ValueKind::String),
                cell(summary_desc, ValueKind::String),
                cell(about, ValueKind::String),
            ]);
        } else if is_prop {
            let kind = if c_name == "Description" {
                explicit_types
                    .first()
                    .map(|t| compact_iri(t, &prefixes))
                    .unwrap_or_else(|| "Property".into())
            } else {
                compact_iri(&tag_type, &prefixes)
            };
            let domain = collect_child_resources(child, "domain")
                .into_iter()
                .map(|s| compact_iri(&s, &prefixes))
                .collect::<Vec<_>>()
                .join(", ");
            let range = collect_child_resources(child, "range")
                .into_iter()
                .map(|s| compact_iri(&s, &prefixes))
                .collect::<Vec<_>>()
                .join(", ");
            prop_rows.push(vec![
                cell(compact_iri(&about, &prefixes), ValueKind::Attribute),
                cell(kind, ValueKind::String),
                cell(domain, ValueKind::String),
                cell(range, ValueKind::String),
                cell(summary_desc, ValueKind::String),
            ]);
        } else if is_concept {
            let broader = collect_child_resources(child, "broader")
                .into_iter()
                .map(|s| compact_iri(&s, &prefixes))
                .collect::<Vec<_>>()
                .join(", ");
            concept_rows.push(vec![
                cell(compact_iri(&about, &prefixes), ValueKind::Attribute),
                cell(label, ValueKind::String),
                cell(broader, ValueKind::String),
                cell(about, ValueKind::String),
            ]);
        } else {
            let ty = if c_name != "Description" {
                compact_iri(&tag_type, &prefixes)
            } else {
                explicit_types
                    .iter()
                    .map(|t| compact_iri(t, &prefixes))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let prop_count = child.children().filter(|n| n.is_element()).count();
            resource_rows.push(vec![
                cell(compact_iri(&about, &prefixes), ValueKind::Attribute),
                cell(ty, ValueKind::String),
                cell(summary_desc, ValueKind::String),
                cell(prop_count.to_string(), ValueKind::Number),
            ]);
        }
    }

    if !ontology_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some("Ontology Metadata".into()),
            depth: 0,
            headers: vec![
                "Ontology".into(),
                "Title / Label".into(),
                "Version".into(),
                "Description".into(),
            ],
            rows: ontology_rows,
        });
    }
    if !class_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some(format!("Classes & Hierarchy ({})", class_rows.len())),
            depth: 0,
            headers: vec![
                "Class".into(),
                "subClassOf".into(),
                "Label / Comment".into(),
                "IRI".into(),
            ],
            rows: class_rows,
        });
    }
    if !prop_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some(format!("Properties ({})", prop_rows.len())),
            depth: 0,
            headers: vec![
                "Property".into(),
                "Kind".into(),
                "Domain".into(),
                "Range".into(),
                "Label / Comment".into(),
            ],
            rows: prop_rows,
        });
    }
    if !concept_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some(format!("SKOS Concepts ({})", concept_rows.len())),
            depth: 0,
            headers: vec![
                "Concept".into(),
                "prefLabel".into(),
                "Broader".into(),
                "IRI".into(),
            ],
            rows: concept_rows,
        });
    }
    if !resource_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some(format!("Resources / Individuals ({})", resource_rows.len())),
            depth: 0,
            headers: vec![
                "Resource".into(),
                "Type".into(),
                "Label / Description".into(),
                "Statements".into(),
            ],
            rows: resource_rows,
        });
    }

    let schema_label = if has_owl {
        "OWL / RDF-XML"
    } else if has_skos {
        "SKOS / RDF-XML"
    } else {
        "RDF/XML"
    };
    let summary = visual::summarize_blocks(
        raw_len,
        fields.len(),
        &blocks,
        Some(schema_label.into()),
        Vec::new(),
    );
    Some((summary, blocks))
}

fn attr_rdf_about<'a>(node: Node<'a, 'a>) -> Option<String> {
    node.attribute((RDF_NS, "about"))
        .map(str::to_string)
        .or_else(|| node.attribute((RDF_NS, "ID")).map(|id| format!("#{id}")))
        .or_else(|| node.attribute((RDF_NS, "nodeID")).map(|id| format!("_:{id}")))
        .or_else(|| node.attribute("about").map(str::to_string))
        .or_else(|| node.attribute("ID").map(|id| format!("#{id}")))
}

fn collect_rdf_types<'a>(node: Node<'a, 'a>) -> Vec<String> {
    let mut out = Vec::new();
    for ch in node.children().filter(|n| n.is_element()) {
        if ch.tag_name().name() == "type" {
            if let Some(res) = ch
                .attribute((RDF_NS, "resource"))
                .or_else(|| ch.attribute("resource"))
            {
                out.push(res.to_string());
            }
        }
    }
    out
}

fn collect_child_resources<'a>(node: Node<'a, 'a>, local_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for ch in node.children().filter(|n| n.is_element()) {
        if ch.tag_name().name() == local_name {
            if let Some(res) = ch
                .attribute((RDF_NS, "resource"))
                .or_else(|| ch.attribute("resource"))
            {
                out.push(res.to_string());
            } else if let Some(txt) = ch.text().map(str::trim).filter(|s| !s.is_empty()) {
                out.push(txt.to_string());
            }
        }
    }
    out
}

fn child_text_first<'a>(node: Node<'a, 'a>, names: &[&str]) -> Option<String> {
    for target in names {
        for ch in node.children().filter(|n| n.is_element()) {
            if ch.tag_name().name() == *target {
                if let Some(txt) = ch.text().map(str::trim).filter(|s| !s.is_empty()) {
                    return Some(txt.to_string());
                }
            }
        }
    }
    None
}

fn child_text_or_resource<'a>(node: Node<'a, 'a>, local_name: &str) -> Option<String> {
    for ch in node.children().filter(|n| n.is_element()) {
        if ch.tag_name().name() == local_name {
            if let Some(res) = ch
                .attribute((RDF_NS, "resource"))
                .or_else(|| ch.attribute("resource"))
            {
                return Some(res.to_string());
            }
            if let Some(txt) = ch.text().map(str::trim).filter(|s| !s.is_empty()) {
                return Some(txt.to_string());
            }
        }
    }
    None
}

// ============================================================================
// Turtle / N-Triples / N-Quads / TriG (.ttl, .nt, .nq, .trig)
// ============================================================================

#[derive(Clone, Debug)]
struct RdfTriple {
    subject: String,
    predicate: String,
    object: String,
    object_kind: ValueKind,
    graph: Option<String>,
}

pub fn read_turtle_family(
    extension: &str,
    src: &str,
    raw_len: usize,
) -> Result<(Vec<Field>, PreviewSummary, Vec<PreviewBlock>), String> {
    let (prefixes, triples) = parse_turtle_triples(src)?;
    let mut fields = Vec::new();
    for t in &triples {
        let p_short = compact_term(&t.predicate, &prefixes);
        let s_short = compact_term(&t.subject, &prefixes);
        let o_short = compact_term(&t.object, &prefixes);
        push_field(&mut fields, &p_short, &format!("{s_short}/{p_short}"), &o_short);
    }

    let mut blocks = Vec::new();
    let mut warnings = Vec::new();

    if !prefixes.is_empty() {
        let ns_rows = prefixes
            .iter()
            .map(|(p, u)| {
                vec![
                    cell(if p.is_empty() { "(default)" } else { p }, ValueKind::Attribute),
                    cell(u, ValueKind::String),
                ]
            })
            .collect();
        blocks.push(PreviewBlock::Table {
            title: Some("Prefixes".into()),
            depth: 0,
            headers: vec!["Prefix".into(), "IRI".into()],
            rows: ns_rows,
        });
    }

    let mut by_subject: BTreeMap<String, Vec<&RdfTriple>> = BTreeMap::new();
    for t in &triples {
        by_subject.entry(t.subject.clone()).or_default().push(t);
    }

    let mut class_rows = Vec::new();
    let mut prop_rows = Vec::new();
    let mut concept_rows = Vec::new();
    let mut graphs: BTreeSet<String> = BTreeSet::new();

    for t in &triples {
        if let Some(g) = &t.graph {
            graphs.insert(compact_term(g, &prefixes));
        }
    }

    for (subj, stmts) in &by_subject {
        let s_short = compact_term(subj, &prefixes);
        let types: Vec<String> = stmts
            .iter()
            .filter(|t| is_rdf_type(&t.predicate))
            .map(|t| compact_term(&t.object, &prefixes))
            .collect();
        let label = find_triple_obj(stmts, &["rdfs:label", "skos:prefLabel", "schema:name", "dc:title", "dcterms:title"], &prefixes);
        let comment = find_triple_obj(stmts, &["rdfs:comment", "skos:definition"], &prefixes);
        let desc = match (label.is_empty(), comment.is_empty()) {
            (false, false) => format!("{label} — {comment}"),
            (false, true) => label.clone(),
            (true, false) => comment.clone(),
            (true, true) => String::new(),
        };

        if types.iter().any(|t| t == "owl:Class" || t == "rdfs:Class") {
            let sub_of = stmts
                .iter()
                .filter(|t| compact_term(&t.predicate, &prefixes) == "rdfs:subClassOf")
                .map(|t| compact_term(&t.object, &prefixes))
                .collect::<Vec<_>>()
                .join(", ");
            class_rows.push(vec![
                cell(s_short, ValueKind::Attribute),
                cell(sub_of, ValueKind::String),
                cell(desc, ValueKind::String),
            ]);
        } else if types.iter().any(|t| t.ends_with("Property")) {
            let domain = stmts
                .iter()
                .filter(|t| compact_term(&t.predicate, &prefixes) == "rdfs:domain")
                .map(|t| compact_term(&t.object, &prefixes))
                .collect::<Vec<_>>()
                .join(", ");
            let range = stmts
                .iter()
                .filter(|t| compact_term(&t.predicate, &prefixes) == "rdfs:range")
                .map(|t| compact_term(&t.object, &prefixes))
                .collect::<Vec<_>>()
                .join(", ");
            prop_rows.push(vec![
                cell(s_short, ValueKind::Attribute),
                cell(types.join(", "), ValueKind::String),
                cell(domain, ValueKind::String),
                cell(range, ValueKind::String),
                cell(desc, ValueKind::String),
            ]);
        } else if types.iter().any(|t| t == "skos:Concept" || t == "skos:ConceptScheme") {
            let broader = stmts
                .iter()
                .filter(|t| compact_term(&t.predicate, &prefixes) == "skos:broader")
                .map(|t| compact_term(&t.object, &prefixes))
                .collect::<Vec<_>>()
                .join(", ");
            concept_rows.push(vec![
                cell(s_short, ValueKind::Attribute),
                cell(label, ValueKind::String),
                cell(broader, ValueKind::String),
            ]);
        }
    }

    let stats_rows = vec![
        vec![cell("Triples", ValueKind::Attribute), cell(triples.len().to_string(), ValueKind::Number)],
        vec![cell("Subjects", ValueKind::Attribute), cell(by_subject.len().to_string(), ValueKind::Number)],
        vec![cell("Classes", ValueKind::Attribute), cell(class_rows.len().to_string(), ValueKind::Number)],
        vec![cell("Properties", ValueKind::Attribute), cell(prop_rows.len().to_string(), ValueKind::Number)],
        vec![cell("Named Graphs", ValueKind::Attribute), cell(graphs.len().to_string(), ValueKind::Number)],
    ];
    blocks.push(PreviewBlock::Table {
        title: Some("RDF Graph Summary".into()),
        depth: 0,
        headers: vec!["Metric".into(), "Count".into()],
        rows: stats_rows,
    });

    if !class_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some(format!("Classes & Hierarchy ({})", class_rows.len())),
            depth: 0,
            headers: vec!["Class".into(), "subClassOf".into(), "Label / Comment".into()],
            rows: class_rows,
        });
    }
    if !prop_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some(format!("Properties ({})", prop_rows.len())),
            depth: 0,
            headers: vec![
                "Property".into(),
                "Kind".into(),
                "Domain".into(),
                "Range".into(),
                "Label / Comment".into(),
            ],
            rows: prop_rows,
        });
    }
    if !concept_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some(format!("SKOS Concepts ({})", concept_rows.len())),
            depth: 0,
            headers: vec!["Concept".into(), "prefLabel".into(), "Broader".into()],
            rows: concept_rows,
        });
    }

    const MAX_SUBJECT_SECTIONS: usize = 60;
    for (subj, stmts) in by_subject.iter().take(MAX_SUBJECT_SECTIONS) {
        let s_short = compact_term(subj, &prefixes);
        let types: Vec<String> = stmts
            .iter()
            .filter(|t| is_rdf_type(&t.predicate))
            .map(|t| compact_term(&t.object, &prefixes))
            .collect();
        blocks.push(PreviewBlock::Section {
            title: s_short,
            badge: if types.is_empty() {
                None
            } else {
                Some(types.join(", "))
            },
            depth: 1,
        });
        let rows = stmts
            .iter()
            .map(|t| {
                let p_short = compact_term(&t.predicate, &prefixes);
                let o_short = compact_term(&t.object, &prefixes);
                vec![
                    cell(p_short, ValueKind::Attribute),
                    cell(o_short, t.object_kind),
                ]
            })
            .collect();
        blocks.push(PreviewBlock::Table {
            title: None,
            depth: 2,
            headers: vec!["Predicate".into(), "Object".into()],
            rows,
        });
    }

    if by_subject.len() > MAX_SUBJECT_SECTIONS {
        let msg = format!(
            "Showing {MAX_SUBJECT_SECTIONS} of {} subjects.",
            by_subject.len()
        );
        warnings.push(msg.clone());
        blocks.push(PreviewBlock::Callout {
            tone: CalloutTone::Warning,
            title: "Subjects truncated".into(),
            body: msg,
        });
    }

    let fmt_label = match extension.to_ascii_lowercase().as_str() {
        "nt" => "N-Triples",
        "nq" => "N-Quads",
        "trig" => "TriG",
        _ => "Turtle",
    };
    let summary = visual::summarize_blocks(
        raw_len,
        fields.len(),
        &blocks,
        Some(fmt_label.into()),
        warnings,
    );
    Ok((fields, summary, blocks))
}

fn is_rdf_type(pred: &str) -> bool {
    pred == "a" || pred == "rdf:type" || pred == format!("<{RDF_NS}type>")
}

fn compact_term(term: &str, prefixes: &[(String, String)]) -> String {
    if term == "a" {
        return "rdf:type".into();
    }
    if term.starts_with('<') && term.ends_with('>') {
        let inner = &term[1..term.len() - 1];
        return compact_iri(inner, prefixes);
    }
    term.to_string()
}

fn find_triple_obj(
    stmts: &[&RdfTriple],
    preds: &[&str],
    prefixes: &[(String, String)],
) -> String {
    for target in preds {
        for t in stmts {
            if compact_term(&t.predicate, prefixes) == *target {
                return compact_term(&t.object, prefixes);
            }
        }
    }
    String::new()
}

// ============================================================================
// Lightweight Turtle / N-Triples / N-Quads / TriG tokenizer & parser
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
enum TtlTok {
    Iri(String),
    Prefixed(String),
    Blank(String),
    Literal(String, ValueKind),
    Word(String),
    Dot,
    Semi,
    Comma,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
}

fn tokenize_turtle(src: &str) -> Vec<TtlTok> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut toks = Vec::new();

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'#' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        match b {
            b'.' => {
                // Could be a decimal number like .5
                if i + 1 < len && bytes[i + 1].is_ascii_digit() {
                    let start = i;
                    i += 1;
                    while i < len && (bytes[i].is_ascii_digit() || matches!(bytes[i], b'e' | b'E' | b'+' | b'-')) {
                        i += 1;
                    }
                    toks.push(TtlTok::Literal(src[start..i].to_string(), ValueKind::Number));
                } else {
                    toks.push(TtlTok::Dot);
                    i += 1;
                }
            }
            b';' => {
                toks.push(TtlTok::Semi);
                i += 1;
            }
            b',' => {
                toks.push(TtlTok::Comma);
                i += 1;
            }
            b'{' => {
                toks.push(TtlTok::LBrace);
                i += 1;
            }
            b'}' => {
                toks.push(TtlTok::RBrace);
                i += 1;
            }
            b'[' => {
                toks.push(TtlTok::LBracket);
                i += 1;
            }
            b']' => {
                toks.push(TtlTok::RBracket);
                i += 1;
            }
            b'(' => {
                toks.push(TtlTok::LParen);
                i += 1;
            }
            b')' => {
                toks.push(TtlTok::RParen);
                i += 1;
            }
            b'<' => {
                let start = i;
                i += 1;
                while i < len && bytes[i] != b'>' && bytes[i] != b'\n' {
                    i += 1;
                }
                if i < len && bytes[i] == b'>' {
                    i += 1;
                }
                toks.push(TtlTok::Iri(src[start..i].to_string()));
            }
            b'"' | b'\'' => {
                let quote = b;
                let is_triple = i + 2 < len && bytes[i + 1] == quote && bytes[i + 2] == quote;
                let mut val = String::new();
                if is_triple {
                    i += 3;
                    while i < len {
                        if i + 2 < len && bytes[i] == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
                            i += 3;
                            break;
                        }
                        if bytes[i] == b'\\' && i + 1 < len {
                            val.push(bytes[i + 1] as char);
                            i += 2;
                        } else {
                            let ch = src[i..].chars().next().unwrap();
                            val.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                } else {
                    i += 1;
                    while i < len {
                        if bytes[i] == quote {
                            i += 1;
                            break;
                        }
                        if bytes[i] == b'\\' && i + 1 < len {
                            val.push(bytes[i + 1] as char);
                            i += 2;
                        } else {
                            let ch = src[i..].chars().next().unwrap();
                            val.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                }
                // Optional @lang or ^^datatype
                let mut suffix = String::new();
                if i < len && bytes[i] == b'@' {
                    let s = i;
                    i += 1;
                    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
                        i += 1;
                    }
                    suffix.push_str(&src[s..i]);
                } else if i + 1 < len && bytes[i] == b'^' && bytes[i + 1] == b'^' {
                    i += 2;
                    if i < len && bytes[i] == b'<' {
                        while i < len && bytes[i] != b'>' {
                            i += 1;
                        }
                        if i < len {
                            i += 1;
                        }
                    } else {
                        while i < len && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'.' | b';' | b',' | b']' | b')') {
                            i += 1;
                        }
                    }
                }
                let display = if suffix.is_empty() {
                    val
                } else {
                    format!("{val} ({suffix})")
                };
                toks.push(TtlTok::Literal(display, ValueKind::String));
            }
            _ => {
                let start = i;
                while i < len
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b'<' | b'>' | b'"' | b'\'' | b';' | b',' | b'{' | b'}' | b'[' | b']' | b'(' | b')' | b'#')
                {
                    if bytes[i] == b'.' {
                        // Allow dots inside numbers or prefixed names only if followed by digit/alnum
                        let next_is_part = i + 1 < len
                            && (bytes[i + 1].is_ascii_alphanumeric() || bytes[i + 1] == b'_');
                        if !next_is_part {
                            break;
                        }
                    }
                    i += 1;
                }
                let word = &src[start..i];
                if word.starts_with("_:") {
                    toks.push(TtlTok::Blank(word.to_string()));
                } else if word == "true" || word == "false" {
                    toks.push(TtlTok::Literal(word.to_string(), ValueKind::Boolean));
                } else if word.parse::<f64>().is_ok() {
                    toks.push(TtlTok::Literal(word.to_string(), ValueKind::Number));
                } else if word.contains(':') && !word.starts_with('@') {
                    toks.push(TtlTok::Prefixed(word.to_string()));
                } else {
                    toks.push(TtlTok::Word(word.to_string()));
                }
            }
        }
    }
    toks
}

fn parse_turtle_triples(src: &str) -> Result<(Vec<(String, String)>, Vec<RdfTriple>), String> {
    let toks = tokenize_turtle(src);
    let mut prefixes: Vec<(String, String)> = Vec::new();
    let mut triples: Vec<RdfTriple> = Vec::new();
    let mut idx = 0;
    let mut bnode_counter = 0usize;
    let mut current_graph: Option<String> = None;

    while idx < toks.len() {
        match &toks[idx] {
            TtlTok::Word(w) if w == "@prefix" || w.eq_ignore_ascii_case("PREFIX") => {
                let has_dot = w == "@prefix";
                idx += 1;
                let pfx = match toks.get(idx) {
                    Some(TtlTok::Prefixed(p)) => {
                        idx += 1;
                        p.trim_end_matches(':').to_string()
                    }
                    _ => String::new(),
                };
                if let Some(TtlTok::Iri(iri)) = toks.get(idx) {
                    let clean = iri.trim_start_matches('<').trim_end_matches('>').to_string();
                    prefixes.push((pfx, clean));
                    idx += 1;
                }
                if has_dot && matches!(toks.get(idx), Some(TtlTok::Dot)) {
                    idx += 1;
                }
            }
            TtlTok::Word(w) if w == "@base" || w.eq_ignore_ascii_case("BASE") => {
                let has_dot = w == "@base";
                idx += 1;
                if matches!(toks.get(idx), Some(TtlTok::Iri(_))) {
                    idx += 1;
                }
                if has_dot && matches!(toks.get(idx), Some(TtlTok::Dot)) {
                    idx += 1;
                }
            }
            TtlTok::Word(w) if w.eq_ignore_ascii_case("GRAPH") => {
                idx += 1;
                if let Some(g) = tok_as_term(toks.get(idx)) {
                    current_graph = Some(g.0);
                    idx += 1;
                }
                if matches!(toks.get(idx), Some(TtlTok::LBrace)) {
                    idx += 1;
                }
            }
            TtlTok::LBrace => {
                idx += 1;
            }
            TtlTok::RBrace => {
                current_graph = None;
                idx += 1;
            }
            TtlTok::Dot => {
                idx += 1;
            }
            _ => {
                let Some(subj) = parse_term_or_bnode(&toks, &mut idx, &mut bnode_counter, &mut triples, current_graph.as_deref()) else {
                    idx += 1;
                    continue;
                };
                // Check for TriG `{ ... }` after graph name
                if matches!(toks.get(idx), Some(TtlTok::LBrace)) {
                    current_graph = Some(subj);
                    idx += 1;
                    continue;
                }
                // Predicate-object list
                loop {
                    if idx >= toks.len() || matches!(toks[idx], TtlTok::Dot | TtlTok::RBrace) {
                        break;
                    }
                    if matches!(toks[idx], TtlTok::Semi) {
                        idx += 1;
                        continue;
                    }
                    let Some((pred, _)) = tok_as_term(toks.get(idx)) else {
                        break;
                    };
                    idx += 1;
                    // Object list
                    loop {
                        let Some(obj) = parse_object_term(&toks, &mut idx, &mut bnode_counter, &mut triples, current_graph.as_deref()) else {
                            break;
                        };
                        // Optional 4th N-Quads graph element before `.`
                        let mut stmt_graph = current_graph.clone();
                        if matches!(toks.get(idx), Some(TtlTok::Iri(_) | TtlTok::Blank(_)))
                            && matches!(toks.get(idx + 1), Some(TtlTok::Dot))
                        {
                            if let Some((g, _)) = tok_as_term(toks.get(idx)) {
                                stmt_graph = Some(g);
                                idx += 1;
                            }
                        }
                        triples.push(RdfTriple {
                            subject: subj.clone(),
                            predicate: pred.clone(),
                            object: obj.0,
                            object_kind: obj.1,
                            graph: stmt_graph,
                        });
                        if matches!(toks.get(idx), Some(TtlTok::Comma)) {
                            idx += 1;
                            continue;
                        }
                        break;
                    }
                    if matches!(toks.get(idx), Some(TtlTok::Semi)) {
                        idx += 1;
                        continue;
                    }
                    break;
                }
                if matches!(toks.get(idx), Some(TtlTok::Dot)) {
                    idx += 1;
                }
            }
        }
    }

    prefixes.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    Ok((prefixes, triples))
}

fn tok_as_term(tok: Option<&TtlTok>) -> Option<(String, ValueKind)> {
    match tok? {
        TtlTok::Iri(s) => Some((s.clone(), ValueKind::Attribute)),
        TtlTok::Prefixed(s) => Some((s.clone(), ValueKind::Attribute)),
        TtlTok::Blank(s) => Some((s.clone(), ValueKind::Attribute)),
        TtlTok::Literal(s, k) => Some((s.clone(), *k)),
        TtlTok::Word(w) if w == "a" => Some(("a".into(), ValueKind::Attribute)),
        _ => None,
    }
}

fn parse_term_or_bnode(
    toks: &[TtlTok],
    idx: &mut usize,
    bnode_counter: &mut usize,
    triples: &mut Vec<RdfTriple>,
    graph: Option<&str>,
) -> Option<String> {
    match toks.get(*idx)? {
        TtlTok::LBracket => {
            *idx += 1;
            *bnode_counter += 1;
            let bnode = format!("_:b{bnode_counter}");
            while *idx < toks.len() && !matches!(toks[*idx], TtlTok::RBracket) {
                if matches!(toks[*idx], TtlTok::Semi) {
                    *idx += 1;
                    continue;
                }
                let Some((pred, _)) = tok_as_term(toks.get(*idx)) else {
                    *idx += 1;
                    continue;
                };
                *idx += 1;
                loop {
                    let Some((obj, kind)) = parse_object_term(toks, idx, bnode_counter, triples, graph) else {
                        break;
                    };
                    triples.push(RdfTriple {
                        subject: bnode.clone(),
                        predicate: pred.clone(),
                        object: obj,
                        object_kind: kind,
                        graph: graph.map(str::to_string),
                    });
                    if matches!(toks.get(*idx), Some(TtlTok::Comma)) {
                        *idx += 1;
                        continue;
                    }
                    break;
                }
            }
            if matches!(toks.get(*idx), Some(TtlTok::RBracket)) {
                *idx += 1;
            }
            Some(bnode)
        }
        TtlTok::LParen => {
            *idx += 1;
            let mut items = Vec::new();
            while *idx < toks.len() && !matches!(toks[*idx], TtlTok::RParen) {
                if let Some((item, _)) = parse_object_term(toks, idx, bnode_counter, triples, graph) {
                    items.push(item);
                } else {
                    *idx += 1;
                }
            }
            if matches!(toks.get(*idx), Some(TtlTok::RParen)) {
                *idx += 1;
            }
            Some(format!("({})", items.join(", ")))
        }
        _ => {
            let (s, _) = tok_as_term(toks.get(*idx))?;
            *idx += 1;
            Some(s)
        }
    }
}

fn parse_object_term(
    toks: &[TtlTok],
    idx: &mut usize,
    bnode_counter: &mut usize,
    triples: &mut Vec<RdfTriple>,
    graph: Option<&str>,
) -> Option<(String, ValueKind)> {
    match toks.get(*idx)? {
        TtlTok::LBracket => {
            let b = parse_term_or_bnode(toks, idx, bnode_counter, triples, graph)?;
            Some((b, ValueKind::Attribute))
        }
        TtlTok::LParen => {
            let list = parse_term_or_bnode(toks, idx, bnode_counter, triples, graph)?;
            Some((list, ValueKind::Container))
        }
        _ => {
            let res = tok_as_term(toks.get(*idx))?;
            *idx += 1;
            Some(res)
        }
    }
}
