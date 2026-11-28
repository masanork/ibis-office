// SPDX-License-Identifier: AGPL-3.0-only
//! Unified read-only Markdown extraction across supported document formats.

use crate::{Error, markdown::export_presentation_to_markdown};

#[derive(Debug, Clone)]
pub struct ExtractedMarkdown {
    pub format: &'static str,
    pub markdown: String,
    pub page_count: Option<usize>,
}

pub fn extract_to_markdown(extension: Option<&str>, data: &[u8]) -> Result<ExtractedMarkdown, Error> {
    let ext = extension.map(str::to_ascii_lowercase);

    if matches!(ext.as_deref(), Some("jtd" | "jtt" | "jttc")) {
        // JTD parsing lives in ibis-jtd, which itself depends on this crate's
        // Error and model types, so it cannot be called from here. Callers
        // that can depend on both crates route JTD through ibis_jtd::read
        // plus markdown::export_presentation_to_markdown (as the doc-service
        // extract tool does), the same split as ibis_formats::read.
        return Err(Error::new(crate::JTD_MESSAGE));
    }

    if matches!(ext.as_deref(), Some("epub")) || crate::epub::is_epub(data) {
        let markdown = crate::epub::to_markdown(data)?;
        return Ok(ExtractedMarkdown {
            format: "epub",
            markdown,
            page_count: None,
        });
    }

    if matches!(ext.as_deref(), Some("hwp")) {
        let opened = crate::hwp::open(data)?;
        let markdown = export_presentation_to_markdown(&opened.presentation)?;
        return Ok(ExtractedMarkdown {
            format: "hwp",
            markdown,
            page_count: None,
        });
    }

    if matches!(ext.as_deref(), Some("hwpx")) {
        let opened = crate::hwpx::open(data)?;
        let markdown = export_presentation_to_markdown(&opened.presentation)?;
        return Ok(ExtractedMarkdown {
            format: "hwpx",
            markdown,
            page_count: None,
        });
    }

    if matches!(ext.as_deref(), Some("wpd" | "wp6")) {
        let opened = crate::wpd::open(data)?;
        let markdown = export_presentation_to_markdown(&opened.presentation)?;
        return Ok(ExtractedMarkdown {
            format: "wpd",
            markdown,
            page_count: None,
        });
    }

    if matches!(ext.as_deref(), Some("uot")) || crate::uot::is_uot(data) {
        let opened = crate::uot::open(data)?;
        let markdown = export_presentation_to_markdown(&opened.presentation)?;
        return Ok(ExtractedMarkdown {
            format: "uot",
            markdown,
            page_count: None,
        });
    }

    if matches!(ext.as_deref(), Some("md" | "markdown")) {
        let markdown = std::str::from_utf8(data)
            .map_err(|_| Error::new("Markdown file is not valid UTF-8"))?
            .to_owned();
        return Ok(ExtractedMarkdown {
            format: "markdown",
            markdown,
            page_count: None,
        });
    }

    let opened = crate::open(data.to_vec())?;
    let format = match crate::Kind::of(&opened.presentation) {
        crate::Kind::Workbook => "workbook",
        crate::Kind::Document => "document",
        crate::Kind::Presentation => "presentation",
        crate::Kind::Pdf => "pdf",
    };
    let page_count = if format == "presentation" {
        Some(opened.presentation.slides.len())
    } else {
        None
    };
    let markdown = export_presentation_to_markdown(&opened.presentation)?;
    Ok(ExtractedMarkdown {
        format,
        markdown,
        page_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_markdown_csv_and_blank_ooxml() {
        let md = extract_to_markdown(Some("md"), b"# Title\n\nBody\n").unwrap();
        assert_eq!(md.format, "markdown");
        assert!(md.markdown.contains("# Title"));

        let csv = extract_to_markdown(Some("csv"), b"code,name\nA01,Tokyo\n").unwrap();
        assert_eq!(csv.format, "workbook");
        assert!(csv.markdown.contains("A01"));

        let docx = crate::blank_file(crate::Kind::Document).unwrap();
        let doc_res = extract_to_markdown(Some("docx"), &docx).unwrap();
        assert_eq!(doc_res.format, "document");

        assert!(extract_to_markdown(Some("jtd"), b"not a jtd").is_err());
    }
}
