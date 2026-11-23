// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust Lawtext / e-Gov 法令標準XML ⇄ Markdown bidirectional converter and
//! 新旧対照表 (side-by-side amendment comparison) generator.
//!
//! Bridges Japanese statutory XML (`<Law>...</Law>`), human-editable Lawtext
//! (inspired by `yamachig/Lawtext`), and ibisOffice's Markdown pipeline so
//! legal drafts can be edited in Markdown/Lawtext and exported to DOCX, PPTX,
//! XLSX, or e-Gov 法令標準XML.

use crate::Error;

pub mod aratame;
pub mod assistant;
pub mod diff;
pub mod lint;
mod print;
pub use print::{comparison_docx, open_comparison_document};
pub mod refs;
pub use aratame::{
    AratameClause, AratameDraft, VerifyResult, apply_aratame, generate_aratame_draft,
    verify_aratame, verify_aratame_with_bill_supplement,
};
pub use assistant::{
    Legislation5SetPackage, build_5set_package, build_5set_package_with_bill_supplement,
    build_5set_package_with_sources,
};
pub use diff::{
    AmendmentIr, ArticleChange, ArticleChangeKind, DiffOp, DivisionChange, DivisionChangeKind,
    ItemChange, NodeChangeKind, ParagraphChange, ShinkyuOptions, SubitemChange, SupplChange,
    diff_laws, generate_shinkyu_markdown, generate_shinkyu_markdown_with_options,
};
pub use lint::{LintFinding, LintReport, LintSeverity, lint_amendment, lint_law};
pub use refs::{LawReference, extract_references, generate_sansho_markdown};

mod model;
mod xml;
mod text;
mod markup;
mod markdown;
mod syntax;
mod table_markup;
pub use model::{
    DivisionKind, LawAppdxTable, LawArticle, LawDoc, LawItem, LawNode, LawParagraph,
    LawParagraphLabels, LawSentence, LawSentenceColumn, LawSentenceContent, LawSubitem,
    LawTitleAttributes, SupplProvision,
};
pub use xml::{parse_law_xml, to_law_xml};
pub use text::{is_lawtext, parse_lawtext, to_lawtext};
pub(crate) use markdown::{is_law_profile, markdown_profile};
pub use markdown::{export_ai_law, parse_markdown_law, parse_markdown_law_with_diagnostics, to_markdown};
pub(crate) use markdown::MARKDOWN_SENTENCE_COMMENT;
use text::write_lawtext_node;
use syntax::{
    is_kanji_digit, kanji_num_to_ascii, parse_article_header, parse_numbered_paragraph,
    split_leading_token, to_fullwidth_digits,
};

/// Parse explicit legislative drafting input as e-Gov XML, law Markdown or Lawtext.
pub fn parse_law_input(text: &str) -> Result<LawDoc, Error> {
    let text = text.trim_start_matches('\u{feff}').trim_start();
    let doc = if text.starts_with('<') {
        parse_law_xml(text)?
    } else if crate::markdown::roundtrip::has_document_type(text, "lawtext") {
        parse_markdown_law(text)?
    } else {
        parse_lawtext(text)?
    };
    if doc.main_provision.is_empty() && doc.suppl_provisions.is_empty() && doc.appdx_tables.is_empty() {
        return Err(Error::new("解析できる本則・附則・別表がありません"));
    }
    Ok(doc)
}

/// Return `true` when `data` looks like an e-Gov 法令標準XML document.
pub fn is_law_xml(data: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(data) else {
        return false;
    };
    let head = &text[..text.floor_char_boundary(4096)];
    head.contains("<Law") && (head.contains("<LawBody") || head.contains("<LawNum"))
}

/// The file `path` as Lawtext-flavoured Markdown, when it is a law and only then: a `.lawtext` or `.law.txt` file (read as Lawtext), or a `.xml` file whose content is
/// e-Gov 法令標準XML (`<Law` ...). Any other file, whatever its content (a CSV or a text file whose first line has （）and which has a line of 第一条), gives `None` and is
/// never looked at, so it opens as it always did; a `.xml` that is not a law (Office 2003 XML and the others) gives `None` too. An `.md` is Markdown already.
pub fn path_input_to_markdown(path: &std::path::Path, data: &[u8]) -> Result<Option<String>, Error> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.ends_with(".lawtext") || name.ends_with(".law.txt") {
        let text = std::str::from_utf8(data).map_err(|_| Error::new("Lawtext が有効な UTF-8 ではありません"))?;
        return parse_lawtext(text).map(|doc| Some(to_markdown(&doc)));
    }
    if name.ends_with(".xml") && is_law_xml(data) {
        let text = std::str::from_utf8(data).map_err(|_| Error::new("法令XMLが有効な UTF-8 ではありません"))?;
        return parse_law_xml(text).map(|doc| Some(to_markdown(&doc)));
    }
    Ok(None)
}

/// If `data` is e-Gov 法令標準XML or Lawtext, convert it to Lawtext-flavoured Markdown (by its content; a caller that has the file's name uses
/// [`path_input_to_markdown`], which does not take a CSV for Lawtext).
pub fn input_to_markdown(data: &[u8]) -> Result<Option<String>, Error> {
    if is_law_xml(data) {
        let text = std::str::from_utf8(data)
            .map_err(|_| Error::new("法令XMLが有効な UTF-8 ではありません"))?;
        return parse_law_xml(text).map(|doc| Some(to_markdown(&doc)));
    }
    if is_lawtext(data) {
        let text = std::str::from_utf8(data)
            .map_err(|_| Error::new("Lawtext が有効な UTF-8 ではありません"))?;
        return parse_lawtext(text).map(|doc| Some(to_markdown(&doc)));
    }
    Ok(None)
}

/// Export an open [`crate::Presentation`] to e-Gov 法令標準XML via its Markdown representation.
pub fn export_presentation_to_law_xml(p: &crate::Presentation) -> Result<String, Error> {
    let md = crate::markdown::export_presentation_to_markdown(p)?;
    let law = parse_markdown_law(&md)?;
    Ok(to_law_xml(&law))
}

/// Export an open [`crate::Presentation`] to Lawtext plain text via its Markdown representation.
pub fn export_presentation_to_lawtext(p: &crate::Presentation) -> Result<String, Error> {
    let md = crate::markdown::export_presentation_to_markdown(p)?;
    let law = parse_markdown_law(&md)?;
    Ok(to_lawtext(&law))
}

#[cfg(test)]
mod tests;
