// SPDX-License-Identifier: AGPL-3.0-only
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::HashSet;
use std::fmt::Write;

use ibis_doc_model::MediaFormat;

use super::{decode_data_uri_image, encode_base64, is_fenced_line};

const START: &str = "<!-- ibis-metafile-previews -->";
const END: &str = "<!-- /ibis-metafile-previews -->";
const HEADING: &str = "## Metafile previews";
const PREVIEW: &str = "![Metafile preview ";
const TARGET: &str = "](data:image/svg+xml;base64,";

pub(super) fn append(markdown: &mut String) {
    let mut previews = String::new();
    let mut seen = HashSet::new();
    let mut fence = None;
    let mut index = 0;
    for line in markdown.lines() {
        if is_fenced_line(line.trim(), &mut fence) {
            continue;
        }
        for (start, _) in line.match_indices("](data:image/") {
            let Some((uri, attributes)) = line[start + 2..].split_once(')') else {
                continue;
            };
            if !attributes.starts_with("{width=") {
                continue;
            }
            let Some(subtype) = uri.strip_prefix("data:image/") else {
                continue;
            };
            if !subtype.starts_with("x-emf;base64,") && !subtype.starts_with("x-wmf;base64,") {
                continue;
            }
            if !seen.insert(uri) {
                continue;
            }
            let Some((format, bytes)) = decode_data_uri_image(uri) else {
                continue;
            };
            let svg = match format {
                MediaFormat::Emf => ibis_emf::emf_to_svg(&bytes),
                MediaFormat::Wmf => ibis_emf::wmf_to_svg(&bytes),
                _ => None,
            };
            let Some(svg) = svg else { continue };
            index += 1;
            let _ = writeln!(
                previews,
                "{PREVIEW}{index}{TARGET}{})\n",
                encode_base64(svg.as_bytes())
            );
            if let Ok(document) = roxmltree::Document::parse(&svg) {
                for text in document
                    .descendants()
                    .filter(|node| node.has_tag_name("text"))
                {
                    let label = text
                        .descendants()
                        .filter(|node| node.is_text())
                        .filter_map(|node| node.text())
                        .collect::<String>();
                    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !label.is_empty() {
                        let _ = writeln!(previews, "> {}", escape_label(&label));
                    }
                }
                if !previews.ends_with("\n\n") {
                    previews.push('\n');
                }
            }
        }
    }
    if !previews.is_empty() {
        let _ = writeln!(markdown, "\n\n{START}\n\n{HEADING}\n\n{previews}{END}");
    }
}

// Quote labels as plain text so characters in a metafile cannot add Markdown or HTML.
fn escape_label(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            let _ = write!(escaped, "&#{};", u32::from(ch));
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

// Drop only a complete generated appendix, and only at the end of the file.
pub(super) fn strip(markdown: &str) -> &str {
    let Some(before_end) = markdown.trim_end().strip_suffix(END) else {
        return markdown;
    };
    let Some((body, appendix)) = before_end.rsplit_once(START) else {
        return markdown;
    };
    if !body.trim_end_matches([' ', '\t']).ends_with('\n') {
        return markdown;
    }
    let mut lines = appendix.lines().filter(|line| !line.trim().is_empty());
    if lines.next() != Some(HEADING) {
        return markdown;
    }
    let mut index = 0;
    for line in lines {
        if line.starts_with("> ") && index > 0 {
            continue;
        }
        index += 1;
        let Some(payload) = line
            .strip_prefix(PREVIEW)
            .and_then(|rest| rest.split_once(TARGET))
            .filter(|(number, _)| number.parse::<usize>() == Ok(index))
            .and_then(|(_, rest)| rest.strip_suffix(')'))
        else {
            return markdown;
        };
        if payload.is_empty()
            || !payload
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
        {
            return markdown;
        }
    }
    // A matching marker in an unterminated code fence belongs to the source.
    let mut fence = None;
    for line in body.lines() {
        is_fenced_line(line.trim(), &mut fence);
    }
    if index > 0 && fence.is_none() {
        body.strip_suffix("\r\n\r\n")
            .or_else(|| body.strip_suffix("\n\n"))
            .unwrap_or(body)
    } else {
        markdown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{
        export_document_markdown_with_metafile_previews, export_presentation_to_markdown,
        import_markdown_to_docx_bytes,
    };

    #[test]
    fn previews_preserve_the_original_media_and_expose_japanese_text() {
        let mut wmf = vec![1, 0, 9, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut record = |kind: u16, data: &[u8]| {
            assert_eq!(data.len() % 2, 0);
            wmf.extend((3 + data.len() as u32 / 2).to_le_bytes());
            wmf.extend(kind.to_le_bytes());
            wmf.extend(data);
        };
        let mut font = vec![0; 20];
        font[..2].copy_from_slice(&(-20i16).to_le_bytes());
        font[13] = 128; // Shift_JIS
        record(0x020C, &[100, 0, 100, 0]);
        record(0x02FB, &font);
        record(0x012D, &[0, 0]);
        record(
            0x0521,
            &[6, 0, 0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA, 10, 0, 5, 0],
        ); // 日本語
        record(0, &[]);
        let emf = include_bytes!("../../../ibis-emf/tests/fixtures/shapes.emf");
        let source = format!(
            "![Japanese](data:image/x-wmf;base64,{}){{width=20mm height=20mm}}\n\n![Drawing](data:image/x-emf;base64,{}){{width=20mm height=20mm}}\n\n![Invalid](data:image/x-emf;base64,YmFk){{width=20mm height=20mm}}",
            encode_base64(&wmf),
            encode_base64(emf)
        );
        let docx = import_markdown_to_docx_bytes(source.as_bytes()).unwrap();
        let presentation = crate::read_docx(&docx).unwrap();
        let plain = export_presentation_to_markdown(&presentation).unwrap();
        let output = export_document_markdown_with_metafile_previews(&presentation).unwrap();
        assert!(
            strip(&output) == plain,
            "appendix stripping changed the original Markdown"
        );
        assert_eq!(output.matches(TARGET).count(), 2);
        assert!(output.contains("> 日本語"));
        let preview = output
            .split_once(TARGET)
            .unwrap()
            .1
            .split_once(')')
            .unwrap()
            .0;
        assert!(
            String::from_utf8(crate::markdown::decode_base64(preview).unwrap())
                .unwrap()
                .contains(">日本語<")
        );
        let rebuilt = import_markdown_to_docx_bytes(output.as_bytes()).unwrap();
        let rebuilt = crate::read_docx(&rebuilt).unwrap();
        assert_eq!(rebuilt.media.len(), 3);
        assert!(
            export_presentation_to_markdown(&rebuilt).unwrap() == plain,
            "round trip changed the original Markdown"
        );
    }

    #[test]
    fn leaves_literal_or_incomplete_appendices_in_the_source() {
        let appendix = format!("{START}\n\n{HEADING}\n\n{PREVIEW}1{TARGET}QQ==)\n\n{END}");
        let examples = [
            format!("```md\n{appendix}"),
            format!("body\n\n{appendix}\n\nMore text."),
            format!("body\n\n{START}\n\n{HEADING}\n\nUnrelated text\n\n{END}"),
        ];
        for source in &examples {
            assert_eq!(strip(source), source);
        }
    }
}
