// SPDX-License-Identifier: AGPL-3.0-only
//! Render the parsed preview as literal, selectable document text.

use ibis_doc_model::{Block, FlowParagraph, FlowRun, FlowRunKind, FlowRunStyle};
use ibis_formats::{Kind, Opened, structured_preview};
use std::sync::Arc;

pub fn open(extension: &str, data: &[u8]) -> Result<Opened, String> {
    let preview = structured_preview::read(extension, data)?;
    let bytes = ibis_formats::blank_file(Kind::Document).map_err(|e| e.to_string())?;
    let mut opened = ibis_formats::open(bytes).map_err(|e| e.to_string())?;
    let flow = opened
        .presentation
        .flow
        .as_mut()
        .ok_or("Cannot create structured preview")?;
    let mut body = Vec::new();
    let mut line = |label: &str, value: &str, path: Option<&str>| {
        let mut runs = vec![FlowRun::new(
            FlowRunKind::Text(literal(label)),
            FlowRunStyle {
                bold: true,
                ..Default::default()
            },
        )];
        if !value.is_empty() {
            runs.push(FlowRun::new(
                FlowRunKind::Text(format!("  {}", literal(value))),
                FlowRunStyle::default(),
            ));
        }
        if let Some(path) = path.filter(|path| *path != label) {
            runs.push(FlowRun::new(
                FlowRunKind::Text(format!("  ·  {}", literal(path))),
                FlowRunStyle {
                    size: 900,
                    color: ibis_doc_model::Rgb(0x595959),
                    ..Default::default()
                },
            ));
        }
        body.push(Block::Paragraph(FlowParagraph {
            id: body.len() as u32 + 1,
            runs,
            ..Default::default()
        }));
    };
    let t = crate::i18n::text;
    line(&t("structured-preview.title"), preview.format, None);
    line(&t("structured-preview.read-only"), "", None);
    if let Some(credential) = preview.credential {
        if let Some(credential_type) = credential.credential_type {
            line(
                &t("structured-preview.credential-type"),
                &credential_type,
                None,
            );
        }
        if let Some(issuer) = credential.issuer {
            line(&t("structured-preview.issuer"), &issuer, None);
        }
        line(&t("structured-preview.received-only"), "", None);
        line(&t("structured-preview.unverified"), "", None);
        line(
            &t("structured-preview.matching-digests"),
            &credential.matching_digests.to_string(),
            None,
        );
    }
    for field in preview.fields {
        line(&field.name, &field.value, Some(&field.path));
    }
    Arc::make_mut(flow).body = body;
    Ok(opened)
}

// Show controls visibly, including bidi controls that can disguise a field’s name.
fn literal(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars();
    for ch in chars.by_ref().take(4_096) {
        if ch.is_control()
            || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            result.extend(ch.escape_default());
        } else {
            result.push(ch);
        }
    }
    if chars.next().is_some() {
        result.push_str(" …");
    }
    result
}
