// SPDX-License-Identifier: AGPL-3.0-only
//! Unified read-only Markdown extraction tool across Office, PDF, and legacy document formats.

use serde_json::{Value, json};

use crate::catalog::{Access, Tool, integer, object, string};
use crate::service::{Args, Service};
use crate::{ToolError, paths};

const EXTRACTABLE_EXTENSIONS: &[&str] = &[
    "pdf", "jtd", "jtt", "jttc", "hwp", "hwpx", "wpd", "wp6", "uot", "epub", "odt", "ott", "ods",
    "ots", "odp", "otp", "docx", "docm", "dotx", "dotm", "doc", "dot", "xlsx", "xlsm", "xlsb",
    "xls", "csv", "tsv", "tab", "pptx", "pptm", "ppsx", "ppsm", "ppt", "md", "markdown",
];

const DEFAULT_MAX_CHARS: usize = 32_000;
const MAX_CHARS_LIMIT: usize = 120_000;

const TOOLS: &[Tool] = &[Tool {
    name: "doc_extract_markdown",
    description: "Extract readable Markdown/text from PDF, Ichitaro (.jtd), HWP/HWPX, WordPerfect (.wpd), UOF (.uot), EPUB, ODF (.odt/.ods/.odp), or Office (.docx/.xlsx/.pptx/.doc/.xls/.ppt) files with character pagination.",
    access: Access::Read,
    returns_content: true,
    schema: &|| {
        object(
            &[
                (
                    "path",
                    string("Path to a local document file under an allowed folder."),
                ),
                (
                    "offset",
                    integer("Starting character offset (default 0). Pass next_offset for the next page."),
                ),
                (
                    "max_chars",
                    integer("Maximum characters to return (default 32000, maximum 120000)."),
                ),
            ],
            &["path"],
        )
    },
    run: extract_markdown,
}];

pub(crate) fn tools() -> &'static [Tool] {
    TOOLS
}

fn extract_markdown(service: &mut Service, args: &Value) -> Result<Value, ToolError> {
    crate::edit::only_known(args, &["path", "offset", "max_chars"])?;
    let path = paths::resolve(&service.allow_dirs, args.str_arg("path")?, EXTRACTABLE_EXTENSIONS, true)?;
    let metadata = std::fs::metadata(&path)
        .map_err(|e| ToolError::new("io_error", e.to_string()))?;
    if !metadata.is_file() {
        return Err(ToolError::invalid("select a regular document file"));
    }
    if metadata.len() > service.limits.max_file_size {
        return Err(ToolError::new(
            "file_too_large",
            "Document exceeds the file size limit",
        ));
    }
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(0);
    let max_chars = args
        .get("max_chars")
        .and_then(Value::as_u64)
        .map(|v| (v as usize).clamp(1, MAX_CHARS_LIMIT))
        .unwrap_or(DEFAULT_MAX_CHARS);

    let data = std::fs::read(&path).map_err(|e| ToolError::new("io_error", e.to_string()))?;
    let ext = path.extension().and_then(|e| e.to_str());

    let (format, full_markdown, page_count) =
        if ext.is_some_and(|e| e.eq_ignore_ascii_case("pdf")) || data.starts_with(b"%PDF-") {
            let mut container = ibis_pdf::PdfContainer::from_bytes(std::sync::Arc::<[u8]>::from(data))
                .map_err(|e| ToolError::new("extract_error", e.to_string()))?;
            let mut page_count = 0usize;
            let mut out = String::new();
            ibis_pdf::interpret::visit_text_scenes(&mut container, |scene| {
                if page_count > 0 {
                    out.push_str("\n\n---\n\n");
                }
                let text = scene.text_index.plain_text();
                out.push_str(text.trim_end());
                page_count += 1;
                Ok(())
            })
            .map_err(|e| ToolError::new("extract_error", e.to_string()))?;
            ("pdf", out, Some(page_count))
        } else if ext.is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "odt" | "ott" | "ods" | "ots" | "odp" | "otp")) {
            let pres = ibis_odf::read(&data)
                .map_err(|e| ToolError::new("extract_error", e.to_string()))?;
            let md = ibis_formats::markdown::export_presentation_to_markdown(&pres)
                .map_err(|e| ToolError::new("extract_error", e.to_string()))?;
            ("odf", md, None)
        } else if ext.is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jtd" | "jtt" | "jttc")) {
            let pres = ibis_jtd::read(&data)
                .map_err(|e| ToolError::new("extract_error", e.to_string()))?;
            let md = ibis_formats::markdown::export_presentation_to_markdown(&pres)
                .map_err(|e| ToolError::new("extract_error", e.to_string()))?;
            ("jtd", md, None)
        } else {
            let extracted = ibis_formats::extract::extract_to_markdown(ext, &data)
                .map_err(|e| ToolError::new("extract_error", e.to_string()))?;
            (extracted.format, extracted.markdown, extracted.page_count)
        };

    let total_chars = full_markdown.chars().count();
    let slice: String = full_markdown
        .chars()
        .skip(offset)
        .take(max_chars)
        .collect();
    let returned_chars = slice.chars().count();
    let next_offset = offset.saturating_add(returned_chars);
    let truncated = next_offset < total_chars;

    let mut out = json!({
        "format": format,
        "total_chars": total_chars,
        "offset": offset,
        "next_offset": if truncated { Value::from(next_offset) } else { Value::Null },
        "truncated": truncated,
        "markdown": slice,
    });
    if let Some(pages) = page_count {
        out["page_count"] = json!(pages);
    }
    Ok(out)
}
