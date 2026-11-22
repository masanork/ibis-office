// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Rich Markdown (`.md`) native reader, writer, and `.docx` converter for `ibisOffice`.
//!
//! Supports:
//! - YAML Frontmatter (`--- ... ---`): document properties (`title`, `subtitle`, `author`, `keywords`, `description`, `company`),
//!   page setup (`size`/`paper`, `orientation`, `margins`, `columns`, `column_separator`, `direction`),
//!   typography (`font`, `east_asian_font`, `font_size`, `style_set`, `paragraph_spacing`),
//!   header & footer (`header`, `footer` with `{page}` and `{filename}` fields),
//!   `watermark`, `page_color`, `line_numbers`, and `track_changes`.
//! - Block elements:
//!   - ATX headings (`#` .. `######`) with optional attribute blocks `{#anchor align=center}`
//!   - Paragraphs with optional `{align=left|center|right|justify|distribute}`
//!   - Blockquotes (`> ...`) and GitHub Alerts (`> [!NOTE]`, `> [!TIP]`, `> [!IMPORTANT]`, `> [!WARNING]`, `> [!CAUTION]`)
//!   - Bullet lists (`- `, `* `, `+ `) and ordered lists (`1. `) with nested levels (`0..8`)
//!   - Task lists (`- [ ] `, `- [x] `) mapped to `FlowRunKind::CheckBox`
//!   - Fenced code blocks (` ```lang ... ``` `) with monospace styling, shading, and border
//!   - Display math (`$$ ... $$` and ` ```math ` / ` ```latex `) via `ibis_doc_model::latex::parse_latex`
//!   - Native Office charts (` ```chart `) with `type`, `title`, `width`, `height`, and CSV or YAML series data
//!   - GFM Pipe tables (`| ... |`) with column alignment (`:---`, `:---:`, `---:`), header shading, and inline formatting
//!   - Page breaks (`\newpage`, `::: pagebreak`, `<!-- pagebreak -->`), column breaks (`\columnbreak`, `::: columnbreak`),
//!     horizontal rules (`---`, `***`), Table of Contents (`[[toc]]`, `::: toc`), and Footnote definitions (`[^id]: ...`)
//! - Inline elements:
//!   - `**bold**`, `*italic*`, `***bold italic***`, `~~strikethrough~~`, `++underline++`, `==highlight==`, `^superscript^`, `~subscript~`, `` `code` ``
//!   - Inline math `$...$`
//!   - Links `[text](url)` and `[text](#bookmark)`
//!   - Footnote references `[^id]`
//!   - Japanese Ruby `{漢字|かんじ}` and `｜漢字《かんじ》`
//!   - Tate-chu-yoko `[25]{.tcy}` and Emphasis marks `《《強調》》` / `[強調]{.em}`
//!   - CriticMarkup Tracked Changes & Comments: `{++insert++}`, `{--delete--}`, `{~~old~>new~~}`, `{>>author: comment<<}`
//!   - Inline Data URI images `![alt](data:image/png;base64,...){width=... height=...}`

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ibis_doc_model::latex::parse_latex;
use ibis_doc_model::{
    AxisKind, Block, BorderStyle, CellDirection, Chart, ChartGroup, ChartText, ChartTitle, ChartType,
    Comment, DataLabels, DocProperties, EmphasisMark, Emu, EmuRect, EmuSize, Fill, FlowAlign, FlowBorder,
    FlowCell, FlowDocument, FlowParagraph, FlowPos, FlowRow, FlowRun, FlowRunKind, FlowRunStyle,
    FlowStyle, FlowStyles, FlowTable, Grouping, LineHeight, LineNumberRestart, LineNumbers, Link,
    LabelPosition, LegendPosition, ListRef, Media, MediaFormat, MediaId, Note, Number, PageSetup, ParaFormat, ParagraphSpacingPreset,
    PictureLook, Presentation, Revision, RevisionKind, Rgb, RowHeight, RubyAlign, RunFormat,
    Series, ShapeId, ShapeKind, Side, StyleKind, StyleSetPreset, TableLook, TableStyleRef,
    VerticalMerge, VerticalPosition, Watermark,
};

use crate::save::pptx_new_chart::NewChartKind;
use crate::{Error, Opened, SaveOptions};

const EMU_PER_PT: i64 = 12_700;
const EMU_PER_INCH: i64 = 914_400;
const EMU_PER_MM: f64 = 36_000.0;
const DEFAULT_CHART_SIZE: EmuSize = EmuSize::new(5_486_400, 3_200_400);

/// Returns `true` when `data` looks like a UTF-8 Markdown document rather than a binary archive or plain CSV.
pub fn is_markdown(data: &[u8]) -> bool {
    if data.starts_with(b"PK\x03\x04") || data.starts_with(b"\xD0\xCF\x11\xE0") || data.starts_with(b"%PDF-") {
        return false;
    }
    if data.contains(&0) {
        return false;
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return false;
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return false;
    }

    // Check YAML frontmatter at top of file.
    if let Some(after_open) = trimmed.strip_prefix("---\n").or_else(|| trimmed.strip_prefix("---\r\n")) {
        if let Some((fm, _rest)) = split_frontmatter_body(after_open) {
            if fm.lines().any(|l| {
                let l = l.trim();
                !l.is_empty()
                    && !l.starts_with('#')
                    && l.split_once(':').is_some_and(|(k, _)| {
                        !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    })
            }) {
                return true;
            }
        }
    }

    // Text of a table (a CSV or a TSV: the same number of commas, or of tabs, in each of its lines) is no Markdown for the markers a cell may start with: a bullet (`- note`, `+81-90-...`, `* x`), a
    // quotation or a number and a point. Only what a table does not have (headings, fences, pipes, emphasis, links) says Markdown then.
    let tabular = looks_tabular(trimmed);
    for line in trimmed.lines().take(200) {
        let l = line.trim_start();
        let weak = l.starts_with("- ") || l.starts_with("* ") || l.starts_with("+ ") || l.starts_with("> ") || l == ">" || is_ordered_list_line(l);
        if tabular && weak {
            continue;
        }
        if let Some(rest) = l.strip_prefix('#') {
            let hashes = 1 + rest.chars().take_while(|&c| c == '#').count();
            if hashes <= 6 && rest[hashes - 1..].starts_with(' ') {
                return true;
            }
        }
        if l.starts_with("```")
            || l.starts_with("> ")
            || l == ">"
            || l.starts_with("- [ ] ")
            || l.starts_with("- [x] ")
            || l.starts_with("- [X] ")
            || l.starts_with("- ")
            || l.starts_with("* ")
            || l.starts_with("+ ")
            || l.starts_with(":::")
            || l.starts_with("[^")
            || l == "\\newpage"
            || l == "\\columnbreak"
            || l == "[[toc]]"
            || markdown_bibliography_field(l).is_some()
            || l.starts_with("$$")
            || is_horizontal_rule(l)
            || is_ordered_list_line(l)
        {
            return true;
        }
        if l.starts_with('|') && l.ends_with('|') && l.len() >= 3 {
            return true;
        }
        if l.contains("**")
            || l.contains("](")
            || l.contains("~~")
            || l.contains("==")
            || l.contains("{++")
            || l.contains("{--")
            || l.contains("{~~")
            || l.contains("{>>")
            || l.contains("[^")
            || has_inline_code_pair(l)
        {
            return true;
        }
    }

    false
}

/// Whether the first lines have the same number (one or more) of commas, or of tabs: the lines of a CSV or a TSV.
fn looks_tabular(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).take(20).collect();
    if lines.len() < 2 {
        return false;
    }
    [',', '\t'].iter().any(|d| {
        let n = lines[0].matches(*d).count();
        n >= 1 && lines.iter().all(|l| l.matches(*d).count() == n)
    })
}

fn is_ordered_list_line(l: &str) -> bool {
    let digits = l.bytes().take_while(|b| b.is_ascii_digit()).count();
    (1..=4).contains(&digits) && l.as_bytes().get(digits) == Some(&b'.') && l.as_bytes().get(digits + 1) == Some(&b' ')
}

fn has_inline_code_pair(l: &str) -> bool {
    if let Some(first) = l.find('`') {
        let rest = &l[first + 1..];
        rest.contains('`')
    } else {
        false
    }
}

fn split_frontmatter_body(after_open: &str) -> Option<(&str, &str)> {
    let mut offset = 0;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" || trimmed == "..." {
            let fm = &after_open[..offset];
            let rest = &after_open[offset + line.len()..];
            return Some((fm, rest));
        }
        offset += line.len();
    }
    None
}

/// Return `true` when the Markdown bytes contain presentation-mode YAML frontmatter
/// (`marp: true`, `slides: true`, `type: presentation`, or `type: slides`).
pub fn is_presentation_markdown(data: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(data) else {
        return false;
    };
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let Some(after_open) = trimmed.strip_prefix("---\n").or_else(|| trimmed.strip_prefix("---\r\n")) else {
        return false;
    };
    let Some((fm, _)) = split_frontmatter_body(after_open) else {
        return false;
    };
    fm.lines().any(|line| {
        let Some((key, value)) = line.split_once(':') else {
            return false;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase();
        matches!(
            (key.as_str(), value.as_str()),
            ("marp", "true")
                | ("slides", "true")
                | ("type", "presentation" | "slides" | "pptx")
        )
    })
}

/// Read a Markdown document into a [`Presentation`] whose `flow` is the rich [`FlowDocument`].
pub fn read_markdown(data: &[u8]) -> Result<Presentation, Error> {
    Ok(open_markdown(data.to_vec())?.presentation)
}

/// Open a Markdown document with a `.docx`-backed [`crate::Source`] so it can be edited and saved
/// either back to `.md` or directly to `.docx` (preserving native charts, math, tables, footnotes, comments, and page setup).
pub fn open_markdown(data: Vec<u8>) -> Result<Opened, Error> {
    open_markdown_with_base_dir(data, None)
}

/// Open a Markdown document resolving relative image paths against `base_dir` (when provided).
pub fn open_markdown_with_base_dir(data: Vec<u8>, base_dir: Option<&std::path::Path>) -> Result<Opened, Error> {
    let text = std::str::from_utf8(&data).map_err(|_| Error::new("Markdown ファイルが UTF-8 ではありません"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut opened = crate::save::open(crate::blank::blank_file(crate::Kind::Document)?)?;
    let baseline_pres = opened.presentation.clone();
    let mut doc = (**opened
        .presentation
        .flow
        .as_ref()
        .ok_or_else(|| Error::new("白紙文書を開けません"))?)
    .clone();

    let mut media_list: Vec<Media> = opened.presentation.media.clone();
    parse_markdown_into(
        text,
        &mut doc,
        &mut media_list,
        Some(&opened.source),
        Some(&baseline_pres),
        base_dir,
    )?;

    let page_size = doc.page.size;
    let document = Arc::new(doc);
    opened.presentation.media = media_list;
    opened.presentation.slide_size = page_size;
    if let Some(slide) = opened.presentation.slides.first_mut() {
        if let Some(shape) = slide.shapes.first_mut() {
            shape.frame = EmuRect::new(0, 0, page_size.width, page_size.height);
            shape.kind = ShapeKind::Page {
                document: document.clone(),
                index: 0,
            };
        }
    }
    opened.presentation.flow = Some(document);
    Ok(opened)
}

/// Re-parse a full Markdown document (including optional YAML frontmatter) into a fresh
/// [`FlowDocument`], preserving the caller's `view_mode` and `interaction_mode` and appending any
/// newly decoded inline/Mermaid/local images to `media`.
pub fn reparse_markdown_document(
    text: &str,
    current_doc: &FlowDocument,
    media: &mut Vec<Media>,
    source: Option<&crate::Source>,
    baseline_pres: Option<&Presentation>,
    base_dir: Option<&std::path::Path>,
) -> Result<FlowDocument, Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let opened = crate::save::open(crate::blank::blank_file(crate::Kind::Document)?)?;
    let fallback_baseline = opened.presentation;
    let mut doc = (**fallback_baseline
        .flow
        .as_ref()
        .ok_or_else(|| Error::new("白紙文書を開けません"))?)
    .clone();
    doc.view_mode = current_doc.view_mode;
    doc.interaction_mode = current_doc.interaction_mode;

    parse_markdown_into(
        text,
        &mut doc,
        media,
        source.or(Some(&opened.source)),
        baseline_pres.or(Some(&fallback_baseline)),
        base_dir,
    )?;
    Ok(doc)
}

/// Parse a Markdown text fragment into [`Block`]s using `doc`'s styles/numbering (e.g. for clipboard paste).
pub fn parse_markdown_fragment_blocks(
    text: &str,
    doc: &mut FlowDocument,
    media: &mut Vec<Media>,
) -> Result<Vec<Block>, Error> {
    let mut ctx = ParseContext {
        next_id: doc.new_id().max(10),
        next_shape_id: doc.new_shape_id().max(10),
        next_media_id: media.iter().map(|m| m.id.0).max().unwrap_or(0) + 1,
        next_comment_id: doc.comments.iter().map(|c| c.id).max().unwrap_or(0) + 1,
        styles: (*doc.styles).clone(),
        bullet_num_id: None,
        ordered_num_id: None,
        footnote_ids: HashMap::new(),
        footnote_order: Vec::new(),
        next_footnote_id: 1,
        comments: Vec::new(),
        media,
        source: None,
        baseline_pres: None,
        base_dir: None,
    };
    let mut footnote_defs: HashMap<String, String> = HashMap::new();
    let mut blocks = parse_blocks(text, &mut ctx, &mut footnote_defs)?;
    doc.styles = Arc::new(ctx.styles);
    resolve_all_blocks(&mut blocks, &doc.styles);
    Ok(blocks)
}

/// Export a slice of [`Block`]s (such as a selection) to Markdown without YAML frontmatter.
pub fn export_blocks_to_markdown(
    blocks: &[Block],
    doc: &FlowDocument,
    media: &[Media],
) -> String {
    let mut out = String::new();
    let comments_by_id: HashMap<u32, &Comment> = doc.comments.iter().map(|c| (c.id, c)).collect();
    let mut prev_was_list = false;
    for block in blocks {
        let is_list = matches!(block, Block::Paragraph(p) if p.list.is_some() || p.runs.first().is_some_and(|r| matches!(r.kind, FlowRunKind::CheckBox { .. })));
        if prev_was_list && !is_list && !out.ends_with("\n\n") {
            out.push('\n');
        }
        serialize_block(block, doc, media, &comments_by_id, &mut out);
        prev_was_list = is_list;
    }
    out.trim_end().to_string()
}

/// Convert Markdown bytes directly into a `.docx` file's bytes.
pub fn import_markdown_to_docx_bytes(data: &[u8]) -> Result<Vec<u8>, Error> {
    let opened = open_markdown(data.to_vec())?;
    opened
        .source
        .save(&opened.presentation, SaveOptions::default())
}

/// Convert Markdown bytes directly into a `.pptx` presentation's bytes.
pub fn import_markdown_to_pptx_bytes(data: &[u8]) -> Result<Vec<u8>, Error> {
    let opened = open_markdown_presentation(data.to_vec())?;
    opened
        .source
        .save(&opened.presentation, SaveOptions::default())
}

/// Convert Markdown bytes (GFM tables) directly into an `.xlsx` workbook's bytes.
pub fn import_markdown_to_xlsx_bytes(data: &[u8]) -> Result<Vec<u8>, Error> {
    let opened = open_markdown_workbook(data.to_vec())?;
    opened
        .source
        .save(&opened.presentation, SaveOptions::default())
}

/// Open a Markdown document's GFM tables as an Excel workbook (`.xlsx` model).
pub fn open_markdown_workbook(data: Vec<u8>) -> Result<Opened, Error> {
    use ibis_doc_model::sheet_ops::{StructureEdit, apply_structure, valid_sheet_name};

    let text = std::str::from_utf8(&data).map_err(|_| Error::new("Markdown ファイルが UTF-8 ではありません"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let date1904 = text.lines().next().is_some_and(|line| line == XLSX_1904_MARKER);
    let (defined_names, text) = parse_markdown_defined_names(text)?;
    let sections = markdown_workbook_sections(text)?;
    let sheet_states: Vec<_> = sections.iter().enumerate().filter_map(|(index, (marker, _))| {
        marker.as_ref().and_then(|marker| marker.state).map(|state| (index, state))
    }).collect();
    let active_tab = sections.iter().position(|(marker, _)| {
        marker.as_ref().is_none_or(|marker| marker.state.is_none())
    }).ok_or_else(|| Error::new("少なくとも1枚の表示シートが必要です"))?;
    let blank = if date1904 {
        crate::csv::blank_workbook_with_1904_dates()?
    } else {
        crate::blank::blank_file(crate::Kind::Workbook)?
    };
    let mut opened = crate::save::open(blank)?;
    let presentation = &mut opened.presentation;
    let template = presentation.slides.first().and_then(ibis_doc_model::sheet_calc::sheet_of_slide)
        .ok_or_else(|| Error::new("白紙のシートが見つかりません"))?.clone();

    for (index, (marker, markdown)) in sections.into_iter().enumerate() {
        let parsed = open_markdown(markdown.as_bytes().to_vec())?;
        let flow = parsed.presentation.flow.as_ref().ok_or_else(|| Error::new("Markdown のパースに失敗しました"))?;
        let mut content = (*template).clone();
        if content.styles.len() < 2 {
            let base = content.styles.first().cloned().unwrap_or_default();
            if content.styles.is_empty() {
                content.styles.push(base.clone());
            }
            content.styles.push(ibis_doc_model::CellStyle { bold: true, ..base });
        }
        let origin = match marker.as_ref().and_then(|marker| marker.origin.as_deref()) {
            Some(origin) => ibis_doc_model::ai_sheet::parse_cell_ref(origin)
                .ok_or_else(|| Error::new(format!("xlsx-sheet の開始セルが不正です: {origin}")))?,
            None => (0, 0),
        };
        content.cells = markdown_workbook_cells(flow, marker.is_some(), &mut content, date1904, origin)?.into();
        if let Some(MarkdownSheetMarker { name, validations, hidden_rows, hidden_columns, .. }) = marker {
            content.validations = validations;
            apply_markdown_hidden_axis(&hidden_rows, &mut content.rows, &mut content.hidden_rows, content.default_row);
            apply_markdown_hidden_axis(&hidden_columns, &mut content.columns, &mut content.hidden_columns, content.default_column);
            if !valid_sheet_name(presentation, &name, (index == 0).then_some(0)) {
                return Err(Error::new(format!("無効または重複したシート名です: {name}")));
            }
            let edit = if index == 0 {
                StructureEdit::RenameSheet { sheet: 0, name }
            } else {
                let slide = presentation.new_slide_id();
                let shape = presentation.new_shape_id();
                StructureEdit::AddSheet { sheet: index, name, slide, shape, content: Arc::new(content.clone()) }
            };
            apply_structure(presentation, &edit);
            Arc::make_mut(presentation.workbook.as_mut().unwrap()).structure.push(edit);
        }
        if index == 0 {
            let shape = presentation.slides[0].shapes.iter_mut().find(|shape| matches!(shape.kind, ShapeKind::Sheet(_)))
                .ok_or_else(|| Error::new("白紙のシートが見つかりません"))?;
            shape.kind = ShapeKind::Sheet(Arc::new(content));
        }
    }
    let workbook = Arc::make_mut(presentation.workbook.as_mut().unwrap());
    let preserve_name_formulas = !defined_names.is_empty();
    if preserve_name_formulas {
        workbook.names = resolve_markdown_defined_names(defined_names, workbook)?;
    }
    workbook.active_tab = active_tab;
    ibis_doc_model::sheet_command::sync_sheet_book_context(presentation);
    if sheet_states.is_empty() && !preserve_name_formulas {
        return Ok(opened);
    }
    let settings = crate::save::WorkbookImportSettings { sheet_states: &sheet_states, preserve_name_formulas };
    let bytes = opened.source.save_with_workbook_import_settings(&opened.presentation, SaveOptions::default(), settings)?;
    crate::save::open(bytes)
}

const XLSX_1904_MARKER: &str = "<!-- xlsx-date-system:1904 -->";
const XLSX_SHEET_MARKER: &str = "<!-- xlsx-sheet ";

const XLSX_DEFINED_NAMES_MARKER: &str = "<!-- xlsx-defined-names ";

struct MarkdownDefinedName {
    name: ibis_doc_model::DefinedName,
    scope: Option<String>,
}

fn parse_markdown_defined_names(text: &str) -> Result<(Vec<MarkdownDefinedName>, &str), Error> {
    let header = match text.split_once('\n') {
        Some((line, rest)) if line.trim_end_matches('\r') == XLSX_1904_MARKER => rest,
        _ => text,
    };
    let header = header.trim_start_matches(['\n', '\r']);
    let Some(line) = header.lines().next() else { return Ok((Vec::new(), text)) };
    let Some(encoded) = line.trim_end().strip_prefix(XLSX_DEFINED_NAMES_MARKER) else {
        return Ok((Vec::new(), text));
    };
    let malformed = || Error::new("xlsx-defined-names の指定が不正です");
    let value: serde_json::Value = serde_json::from_str(encoded.strip_suffix(" -->").ok_or_else(malformed)?)
        .map_err(|_| malformed())?;
    let names = value.as_array().ok_or_else(malformed)?.iter()
        .map(|value| parse_markdown_defined_name(value).ok_or_else(malformed))
        .collect::<Result<Vec<_>, _>>()?;
    let (_, body) = header.split_once('\n').unwrap_or((header, ""));
    Ok((names, body))
}

fn parse_markdown_defined_name(value: &serde_json::Value) -> Option<MarkdownDefinedName> {
    if value.as_object()?.keys().any(|key| !matches!(key.as_str(), "name" | "formula" | "sheet" | "hidden")) {
        return None;
    }
    let name = value["name"].as_str()?;
    let first = name.chars().next()?;
    if !(first.is_alphabetic() || matches!(first, '_' | '\\'))
        || name.chars().count() > 255
        || ibis_calc::formula::parser::parse_cell(name).is_some()
        || matches!(name.to_ascii_uppercase().as_str(), "TRUE" | "FALSE" | "R" | "C")
        || !name.chars().all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.' | '\\' | '?'))
    {
        return None;
    }
    let formula = value["formula"].as_str()?;
    if formula.starts_with('=') || !is_xml_text(formula) {
        return None;
    }
    let scope = match value.get("sheet") {
        None => None,
        Some(scope) => Some(scope.as_str()?.to_owned()),
    };
    let hidden = match value.get("hidden") {
        None => false,
        Some(hidden) => hidden.as_bool()?,
    };
    Some(MarkdownDefinedName {
        name: ibis_doc_model::DefinedName { name: name.to_owned(), sheet: None, formula: formula.to_owned(), hidden },
        scope,
    })
}

fn resolve_markdown_defined_names(
    names: Vec<MarkdownDefinedName>,
    workbook: &ibis_doc_model::Workbook,
) -> Result<Vec<ibis_doc_model::DefinedName>, Error> {
    let mut existing = std::collections::HashSet::new();
    names.into_iter().map(|entry| {
        let mut name = entry.name;
        if let Some(scope) = entry.scope {
            name.sheet = Some(workbook.sheet_index(&scope)
                .ok_or_else(|| Error::new(format!("定義名のシートが見つかりません: {scope}")))?);
        }
        if !existing.insert((name.name.to_lowercase(), name.sheet)) {
            return Err(Error::new(format!("定義名が重複しています: {}", name.name)));
        }
        Ok(name)
    }).collect()
}

fn workbook_markdown_names(
    p: &Presentation,
    workbook: &ibis_doc_model::Workbook,
    options: WorkbookMarkdownOptions,
) -> Result<Vec<serde_json::Value>, Error> {
    let mut names = Vec::new();
    for name in &workbook.names {
        let mut entry = serde_json::json!({"name": name.name, "formula": name.formula, "hidden": name.hidden});
        if let Some(index) = name.sheet {
            let Some(sheet) = workbook.sheets.get(index) else { continue };
            if workbook_markdown_sheet(p, sheet, options.include_hidden_sheets).is_none() {
                continue;
            }
            entry["sheet"] = serde_json::json!(sheet.name);
        }
        if parse_markdown_defined_name(&entry).is_none() {
            return Err(Error::new(format!("定義名を Markdown に変換できません: {}", name.name)));
        }
        names.push(entry);
    }
    Ok(names)
}

fn is_xml_text(value: &str) -> bool {
    !value.chars().any(|ch| {
        matches!(ch, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}')
    })
}

struct MarkdownSheetMarker {
    name: String,
    origin: Option<String>,
    state: Option<ibis_doc_model::SheetState>,
    validations: Vec<ibis_doc_model::Validation>,
    hidden_rows: Vec<(u32, u32)>,
    hidden_columns: Vec<(u32, u32)>,
}

fn parse_markdown_sheet_marker(encoded: &str) -> Option<MarkdownSheetMarker> {
    let value: serde_json::Value = serde_json::from_str(encoded).ok()?;
    if let Some(name) = value.as_str() {
        return Some(MarkdownSheetMarker {
            name: name.to_owned(), origin: None, state: None, validations: Vec::new(),
            hidden_rows: Vec::new(), hidden_columns: Vec::new(),
        });
    }
    let object = value.as_object()?;
    if object.keys().any(|key| !matches!(key.as_str(), "name" | "origin" | "state" | "validations" | "hiddenRows" | "hiddenColumns")) {
        return None;
    }
    let name = object.get("name")?.as_str()?.to_owned();
    let origin = match object.get("origin") {
        Some(value) => Some(value.as_str()?.to_owned()),
        None => None,
    };
    let state = match object.get("state").map(serde_json::Value::as_str) {
        None | Some(Some("visible")) => None,
        Some(Some("hidden")) => Some(ibis_doc_model::SheetState::Hidden),
        Some(Some("veryHidden")) => Some(ibis_doc_model::SheetState::VeryHidden),
        _ => return None,
    };
    let validations = match object.get("validations") {
        Some(value) => value.as_array()?.iter().map(parse_markdown_validation).collect::<Option<_>>()?,
        None => Vec::new(),
    };
    let hidden_rows = parse_markdown_hidden_axis(object.get("hiddenRows"), |label| {
        label.parse::<u32>().ok()?.checked_sub(1).filter(|&row| row <= ibis_calc::formula::MAX_ROW)
    })?;
    let hidden_columns = parse_markdown_hidden_axis(object.get("hiddenColumns"), |label| {
        (label.len() <= 3).then(|| ibis_calc::formula::parser::column_index(label)).flatten()
    })?;
    Some(MarkdownSheetMarker { name, origin, state, validations, hidden_rows, hidden_columns })
}

fn parse_markdown_hidden_axis(
    value: Option<&serde_json::Value>,
    index: impl Fn(&str) -> Option<u32>,
) -> Option<Vec<(u32, u32)>> {
    let Some(value) = value else { return Some(Vec::new()) };
    let mut ranges = Vec::new();
    for range in value.as_array()? {
        let (first, last) = range.as_str()?.split_once(':')?;
        let (first, last) = (index(first)?, index(last)?);
        if first > last || ranges.last().is_some_and(|&(_, previous_last)| previous_last >= first) {
            return None;
        }
        ranges.push((first, last));
    }
    Some(ranges)
}

fn apply_markdown_hidden_axis(
    ranges: &[(u32, u32)],
    sizes: &mut Vec<ibis_doc_model::Emu>,
    hidden: &mut std::collections::BTreeMap<u32, ibis_doc_model::Emu>,
    default_size: ibis_doc_model::Emu,
) {
    let Some(&(_, last)) = ranges.last() else { return };
    sizes.resize(sizes.len().max(last as usize + 1), default_size);
    for &(first, last) in ranges {
        for index in first..=last {
            let size = &mut sizes[index as usize];
            hidden.insert(index, *size);
            *size = 0;
        }
    }
}

fn parse_markdown_validation(value: &serde_json::Value) -> Option<ibis_doc_model::Validation> {
    let ranges = value["ranges"].as_array()?.iter().map(|range| {
        ibis_doc_model::ai_sheet::parse_cell_range(range.as_str()?)
    }).collect::<Option<Vec<_>>>()?;
    let kind = value["type"].as_str()?;
    let operator = value["operator"].as_str()?;
    if ranges.is_empty()
        || !matches!(kind, "none" | "whole" | "decimal" | "list" | "date" | "time" | "textLength" | "custom")
        || !matches!(operator, "between" | "notBetween" | "equal" | "notEqual" | "lessThan" | "lessThanOrEqual" | "greaterThan" | "greaterThanOrEqual")
    {
        return None;
    }
    let rule = ibis_doc_model::Validation {
        ranges,
        kind: kind.to_owned(),
        operator: operator.to_owned(),
        formula1: serde_json::from_value(value["formula1"].clone()).ok()?,
        formula2: serde_json::from_value(value["formula2"].clone()).ok()?,
        allow_blank: value["allowBlank"].as_bool()?,
        dropdown: value["dropdown"].as_bool()?,
        prompt: serde_json::from_value(value["prompt"].clone()).ok()?,
        error: serde_json::from_value(value["error"].clone()).ok()?,
    };
    let text = rule.formula1.iter().chain(&rule.formula2)
        .chain(rule.prompt.iter().flat_map(|(title, message)| [title, message]))
        .chain(rule.error.iter().flat_map(|(_, title, message)| [title, message]));
    if rule.error.as_ref().is_some_and(|(style, _, _)| !matches!(style.as_str(), "stop" | "warning" | "information"))
        || text.into_iter().any(|value| !is_xml_text(value))
    {
        return None;
    }
    Some(rule)
}

fn validation_range_label(range: &ibis_doc_model::CellRange) -> String {
    use ibis_calc::formula::{MAX_COLUMN, MAX_ROW};
    use ibis_doc_model::cell_name;

    let first = cell_name(range.first_column, range.first_row);
    let last = cell_name(range.last_column.min(MAX_COLUMN), range.last_row.min(MAX_ROW));
    if first == last { first } else { format!("{first}:{last}") }
}

fn markdown_validation(rule: &ibis_doc_model::Validation) -> serde_json::Value {
    serde_json::json!({
        "ranges": rule.ranges.iter().map(validation_range_label).collect::<Vec<_>>(),
        "type": rule.kind,
        "operator": rule.operator,
        "formula1": rule.formula1,
        "formula2": rule.formula2,
        "allowBlank": rule.allow_blank,
        "dropdown": rule.dropdown,
        "prompt": rule.prompt,
        "error": rule.error,
    })
}

fn markdown_workbook_sections(text: &str) -> Result<Vec<(Option<MarkdownSheetMarker>, &str)>, Error> {
    let mut sections = Vec::new();
    let mut current = None;
    let mut fence = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end();
        let fenced = is_fenced_line(trimmed, &mut fence);
        if !fenced && trimmed.starts_with(XLSX_DEFINED_NAMES_MARKER) {
            return Err(Error::new("xlsx-defined-names はブックの先頭に1回だけ指定できます"));
        }
        if !fenced && let Some(encoded) = trimmed.strip_prefix(XLSX_SHEET_MARKER)
        {
            let marker = encoded.strip_suffix(" -->").and_then(parse_markdown_sheet_marker)
                .ok_or_else(|| Error::new("xlsx-sheet の指定が不正です"))?;
            if let Some((previous, start)) = current.replace((marker, offset + line.len())) {
                sections.push((Some(previous), &text[start..offset]));
            }
        }
        offset += line.len();
    }
    if let Some((name, start)) = current {
        sections.push((Some(name), &text[start..]));
    } else {
        sections.push((None, text));
    }
    Ok(sections)
}

const XLSX_TEXT_CELL_MARKER: &str = "<!-- xlsx-text -->";

const XLSX_FORMULA_CELL_MARKER: &str = "<!-- xlsx-formula:";
const XLSX_FORMAT_CELL_MARKER: &str = "<!-- xlsx-format:";
const XLSX_NUMBER_CELL_MARKER: &str = "<!-- xlsx-number:";
const XLSX_CELL_MARKER_END: &str = " -->";

fn push_xlsx_string_marker(out: &mut String, marker: &str, value: &str) {
    push_xlsx_json_marker(out, marker, &serde_json::Value::from(value));
}

fn push_xlsx_json_marker(out: &mut String, marker: &str, value: &serde_json::Value) {
    let encoded = value.to_string();
    out.push(' ');
    out.push_str(marker);
    out.push_str(&encoded.replace('|', r"\u007c").replace('<', r"\u003c").replace('>', r"\u003e").replace("--", r"-\u002d"));
    out.push_str(XLSX_CELL_MARKER_END);
}

fn numeric_cell_marker(text: &str) -> Option<(&str, &str)> {
    text.strip_suffix(XLSX_CELL_MARKER_END)?.rsplit_once(XLSX_NUMBER_CELL_MARKER)
}

fn infer_markdown_cell_value(text: &str) -> ibis_doc_model::CellValue {
    use ibis_doc_model::CellValue;
    if let Ok(number) = text.replace(',', "").parse::<f64>() {
        CellValue::Number(number)
    } else if text.eq_ignore_ascii_case("true") {
        CellValue::Bool(true)
    } else if text.eq_ignore_ascii_case("false") {
        CellValue::Bool(false)
    } else {
        CellValue::Text(text.to_owned())
    }
}

fn markdown_workbook_cells(
    flow: &FlowDocument,
    named: bool,
    sheet: &mut ibis_doc_model::Sheet,
    date1904: bool,
    (origin_row, origin_column): (u32, u32),
) -> Result<Vec<ibis_doc_model::SheetCell>, Error> {
    use ibis_doc_model::{CellError, CellKind, CellRange, CellValue, SheetCell};

    let styles = &mut sheet.styles;

    let mut cells: Vec<SheetCell> = Vec::new();
    let mut formats = std::collections::HashMap::new();
    let mut cur_row: u32 = 0;
    let mut pending_heading: Option<String> = None;
    let mut table_count = 0usize;

    for block in &flow.body {
        match block {
            Block::Paragraph(p) => {
                if !named && p.style.as_deref().is_some_and(|s| s.starts_with("Heading")) {
                    let t = flow_paragraph_plain_text(p);
                    if !t.trim().is_empty() {
                        pending_heading = Some(t.trim().to_string());
                    }
                }
            }
            Block::Table(tbl) => {
                if table_count > 0 {
                    cur_row += 1; // blank separator row between tables
                }
                if let Some(h) = pending_heading.take() {
                    if table_count > 0 || tbl.rows.len() > 1 {
                        let val = CellValue::Text(h.clone());
                        cells.push(SheetCell {
                            row: cur_row,
                            column: 0,
                            kind: CellKind::of(&val),
                            value: val,
                            text: h,
                            style: 1,
                            formula: None,
                            phonetic: None,
                            runs: None,
                        });
                        cur_row += 1;
                    }
                }
                for (ri, row) in tbl.rows.iter().enumerate() {
                    for (ci, cell) in row.cells.iter().enumerate() {
                        let txt: String = cell
                            .blocks
                            .iter()
                            .filter_map(|b| match b {
                                Block::Paragraph(p) => Some(flow_paragraph_plain_text(p)),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        let mut trimmed = txt.trim();
                        let mut formula = None;
                        let mut spill_end = None;
                        let mut fixed_end = None;
                        if named && !trimmed.ends_with(XLSX_TEXT_CELL_MARKER)
                            && let Some((display, encoded)) = trimmed.strip_suffix(XLSX_CELL_MARKER_END)
                            .and_then(|text| text.rsplit_once(XLSX_FORMULA_CELL_MARKER))
                            .filter(|(_, encoded)| !encoded.contains(XLSX_CELL_MARKER_END))
                        {
                            let invalid = || Error::new("xlsx-formula の数式または配列範囲が不正です");
                            let value: serde_json::Value = serde_json::from_str(encoded.trim()).map_err(|_| invalid())?;
                            let expression = if let Some(object) = value.as_object().filter(|object| object.len() == 2) {
                                let end = |key| object.get(key).and_then(serde_json::Value::as_str)
                                    .and_then(ibis_doc_model::ai_sheet::parse_cell_ref);
                                spill_end = end("spill");
                                fixed_end = end("fixedArray");
                                if spill_end.is_none() && fixed_end.is_none() {
                                    return Err(invalid());
                                }
                                object.get("formula").and_then(serde_json::Value::as_str)
                            } else {
                                value.as_str()
                            };
                            formula = Some(expression.and_then(|text| text.strip_prefix('='))
                                .filter(|text| !text.trim().is_empty()).ok_or_else(invalid)?.to_owned());
                            trimmed = display.trim_end();
                        }
                        let mut style = usize::from(ri == 0);
                        let mut number_format = None;
                        if named && !trimmed.ends_with(XLSX_TEXT_CELL_MARKER)
                            && let Some((display, encoded)) = trimmed.strip_suffix(XLSX_CELL_MARKER_END)
                                .and_then(|text| text.rsplit_once(XLSX_FORMAT_CELL_MARKER))
                                .filter(|(_, encoded)| !encoded.contains(XLSX_CELL_MARKER_END))
                        {
                            let code = serde_json::from_str::<String>(encoded.trim()).ok()
                                .filter(|code| !code.trim().is_empty() && !code.chars().any(char::is_control))
                                .ok_or_else(|| Error::new("xlsx-format の表示形式が不正です"))?;
                            trimmed = display.trim_end();
                            number_format = Some(code);
                        }
                        let explicit_text = named.then(|| trimmed.strip_suffix(XLSX_TEXT_CELL_MARKER)).flatten();
                        let (trimmed, value) = if let Some(text) = explicit_text {
                            let text = text.trim_end();
                            (text, CellValue::Text(text.to_owned()))
                        } else if let Some((display, number)) = named.then(|| numeric_cell_marker(trimmed)).flatten() {
                            let value = number.trim().parse::<f64>().ok().filter(|value| value.is_finite())
                                .ok_or_else(|| Error::new(format!("xlsx-number の数値が不正です: {number}")))?;
                            (display.trim_end(), CellValue::Number(value))
                        } else if spill_end.is_some() && trimmed == "#SPILL!" {
                            (trimmed, CellValue::Error(CellError::Spill))
                        } else {
                            if trimmed.is_empty() {
                                if formula.is_none() {
                                    continue;
                                }
                                (trimmed, CellValue::Empty)
                            } else {
                                (trimmed, infer_markdown_cell_value(trimmed))
                            }
                        };
                        let text = if let Some(code) = number_format {
                            let display = crate::xlsx::display(&value, &code, date1904);
                            style = *formats.entry((style, code)).or_insert_with_key(|(base, code)| {
                                let index = styles.len();
                                styles.push(ibis_doc_model::CellStyle {
                                    number_format: code.clone(),
                                    ..styles[*base].clone()
                                });
                                index
                            });
                            display
                        } else {
                            trimmed.to_string()
                        };
                        let row = cur_row.checked_add(origin_row).filter(|&value| value <= ibis_calc::formula::MAX_ROW)
                            .ok_or_else(|| Error::new("Markdown の表がワークシートの行範囲を超えています"))?;
                        let column = (ci as u32).checked_add(origin_column).filter(|&value| value <= ibis_calc::formula::MAX_COLUMN)
                            .ok_or_else(|| Error::new("Markdown の表がワークシートの列範囲を超えています"))?;
                        if let Some((last_row, last_column)) = spill_end {
                            if last_row < row || last_column < column
                                || (value == CellValue::Error(CellError::Spill) && (last_row, last_column) != (row, column))
                            {
                                return Err(Error::new("xlsx-formula のスピル範囲が不正です"));
                            }
                            sheet.spills.insert((row, column), CellRange {
                                first_row: row, first_column: column, last_row, last_column,
                            });
                        }
                        if let Some((last_row, last_column)) = fixed_end {
                            if last_row < row || last_column < column {
                                return Err(Error::new("xlsx-formula の固定配列範囲が不正です"));
                            }
                            sheet.fixed_arrays.insert((row, column), CellRange {
                                first_row: row, first_column: column, last_row, last_column,
                            });
                        }
                        cells.push(SheetCell {
                            row,
                            column,
                            kind: CellKind::of(&value),
                            value,
                            text,
                            style: style as u32,
                            formula,
                            phonetic: None,
                            runs: None,
                        });
                    }
                    cur_row += 1;
                }
                table_count += 1;
            }
        }
    }

    if !sheet.fixed_arrays.is_empty() {
        validate_markdown_array_ranges(&cells, sheet)?;
    }
    Ok(cells)
}

fn validate_markdown_array_ranges(
    cells: &[ibis_doc_model::SheetCell],
    sheet: &ibis_doc_model::Sheet,
) -> Result<(), Error> {
    use ibis_doc_model::CellRange;
    use std::{cmp::Reverse, collections::{BTreeMap, BinaryHeap}};

    // Each ordinary formula occupies just its own cell. Include it so
    // an imported array cannot claim a second formula as a cached result.
    let ranges = cells.iter().filter(|cell| cell.formula.is_some()).map(|cell| {
        let (row, column) = (cell.row, cell.column);
        sheet.fixed_arrays.get(&(row, column)).or_else(|| sheet.spills.get(&(row, column)))
            .copied().unwrap_or(CellRange {
                first_row: row, first_column: column, last_row: row, last_column: column,
            })
    });
    let mut columns = BTreeMap::<u32, u32>::new();
    let mut expires = BinaryHeap::<Reverse<(u32, u32)>>::new();
    // Markdown tables yield cells in row-major order.
    for range in ranges {
        while let Some(&Reverse((last_row, start_column))) = expires.peek() {
            if last_row >= range.first_row {
                break;
            }
            expires.pop();
            columns.remove(&start_column);
        }
        if columns.range(..=range.last_column).next_back()
            .is_some_and(|(_, &end)| end >= range.first_column)
        {
            return Err(Error::new("配列数式の範囲が別の数式と重複しています"));
        }
        columns.insert(range.first_column, range.last_column);
        expires.push(Reverse((range.last_row, range.first_column)));
    }
    Ok(())
}

/// Export a `.docx` file's bytes to deterministic Rich Markdown.
pub fn export_docx_to_markdown(docx_bytes: &[u8]) -> Result<String, Error> {
    let pres = crate::read_docx(docx_bytes)?;
    export_presentation_to_markdown(&pres)
}

/// Export a [`Presentation`] (Word `flow`, Excel `workbook`, or PowerPoint `slides`) to deterministic Rich Markdown.
pub fn export_presentation_to_markdown(presentation: &Presentation) -> Result<String, Error> {
    if let Some(flow) = presentation.flow.as_ref() {
        return Ok(export_markdown(flow, &presentation.media));
    }
    if presentation.workbook.is_some() {
        return export_workbook_markdown(presentation);
    }
    if !presentation.slides.is_empty() {
        return Ok(export_presentation_markdown(presentation));
    }
    Err(Error::new("ワード文書・表計算・プレゼンテーションのいずれでもありません"))
}

/// Export visible worksheets in a workbook [`Presentation`] as GFM Markdown tables.
pub fn export_workbook_markdown(p: &Presentation) -> Result<String, Error> {
    export_workbook_markdown_with_options(p, WorkbookMarkdownOptions::default())
}

/// Workbook metadata explicitly included in a Markdown export.
#[derive(Clone, Copy, Default)]
pub struct WorkbookMarkdownOptions {
    pub include_validations: bool,
    pub include_hidden_axes: bool,
    pub include_hidden_sheets: bool,
    pub include_defined_names: bool,
}

/// Export visible worksheets with the requested metadata for XLSX reconstruction.
pub fn export_workbook_markdown_with_options(
    p: &Presentation,
    options: WorkbookMarkdownOptions,
) -> Result<String, Error> {
    export_workbook_markdown_with_positions(p, options, |_, _, _, _, _, _| {})
}

fn push_workbook_metadata(out: &mut String, value: &serde_json::Value) {
    out.push_str(&value.to_string().replace('<', r"\u003c").replace('>', r"\u003e").replace("--", r"-\u002d"));
    out.push_str(" -->\n\n");
}

fn workbook_markdown_sheet<'a>(
    p: &'a Presentation,
    entry: &'a ibis_doc_model::WorkbookSheet,
    include_hidden: bool,
) -> Option<&'a ibis_doc_model::Sheet> {
    if entry.other {
        return None;
    }
    if entry.state == ibis_doc_model::SheetState::Visible {
        return entry.slide.and_then(|id| p.slide(id)).and_then(ibis_doc_model::sheet_calc::sheet_of_slide).map(|sheet| &**sheet);
    }
    if include_hidden { entry.hidden.as_deref() } else { None }
}

fn export_workbook_markdown_with_positions(
    p: &Presentation,
    options: WorkbookMarkdownOptions,
    mut record: impl FnMut(&str, u32, u32, Option<&ibis_doc_model::SheetCell>, usize, usize),
) -> Result<String, Error> {
    use ibis_doc_model::{CellAlign, SheetState};
    let book = p
        .workbook
        .as_ref()
        .ok_or_else(|| Error::new("ワークブックデータがありません"))?;
    let mut out = String::new();
    if book.date1904 {
        out.push_str(XLSX_1904_MARKER);
        out.push('\n');
    }
    if options.include_defined_names {
        let names = workbook_markdown_names(p, book, options)?;
        if !names.is_empty() {
            out.push_str(XLSX_DEFINED_NAMES_MARKER);
            push_workbook_metadata(&mut out, &serde_json::json!(names));
        }
    }
    let mut total_cells = 0u64;

    let mut has_visible_sheet = false;
    let mut has_hidden_sheet = false;
    for sheet_ref in &book.sheets {
        let Some(sheet) = workbook_markdown_sheet(p, sheet_ref, options.include_hidden_sheets) else {
            continue;
        };
        let state = match sheet_ref.state {
            SheetState::Visible => {
                has_visible_sheet = true;
                None
            }
            SheetState::Hidden => Some("hidden"),
            SheetState::VeryHidden => Some("veryHidden"),
        };
        has_hidden_sheet |= state.is_some();
        let row_vis = |r: u32| {
            options.include_hidden_axes
                || (!sheet.hidden_rows.contains_key(&r) && sheet.rows.get(r as usize).is_none_or(|v| *v != 0))
        };
        let col_vis = |c: u32| {
            options.include_hidden_axes || (!sheet.hidden_columns.contains_key(&c)
                && sheet.columns.get(c as usize).is_none_or(|v| *v != 0))
        };
        let occupied = sheet.cells.iter().filter(|cell| row_vis(cell.row) && col_vis(cell.column));
        let bounds = occupied
            .map(|cell| (cell.row, cell.row, cell.column, cell.column))
            .reduce(|(minr, maxr, minc, maxc), (row, _, column, _)| {
                (minr.min(row), maxr.max(row), minc.min(column), maxc.max(column))
            });
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(XLSX_SHEET_MARKER);
        let origin = bounds
            .filter(|&(row, _, column, _)| (row, column) != (0, 0))
            .map(|(row, _, column, _)| ibis_doc_model::cell_name(column, row));
        let mut metadata = serde_json::Map::new();
        if let Some(state) = state {
            metadata.insert("state".to_owned(), serde_json::json!(state));
        }
        if let Some(origin) = origin {
            metadata.insert("origin".to_owned(), serde_json::json!(origin));
        }
        if options.include_validations && !sheet.validations.is_empty() {
            let rules: Vec<_> = sheet.validations.iter().map(markdown_validation).collect();
            if rules.iter().any(|rule| parse_markdown_validation(rule).is_none()) {
                return Err(Error::new(format!("シート {} の入力規則を Markdown に変換できません", sheet_ref.name)));
            }
            metadata.insert("validations".to_owned(), serde_json::json!(rules));
        }
        if options.include_hidden_axes {
            let (rows, columns) = workbook_hidden_axis_ranges(sheet);
            if !rows.is_empty() {
                metadata.insert("hiddenRows".to_owned(), serde_json::json!(rows));
            }
            if !columns.is_empty() {
                metadata.insert("hiddenColumns".to_owned(), serde_json::json!(columns));
            }
        }
        let marker = if metadata.is_empty() {
            serde_json::json!(sheet_ref.name)
        } else {
            metadata.insert("name".to_owned(), serde_json::json!(sheet_ref.name));
            serde_json::Value::Object(metadata)
        };
        push_workbook_metadata(&mut out, &marker);
        out.push_str("## ");
        out.push_str(&sheet_ref.name);
        out.push_str("\n\n");

        let Some((minr, maxr, minc, maxc)) = bounds else {
            continue;
        };
        let area = u64::from(maxr - minr + 1).saturating_mul(u64::from(maxc - minc + 1));
        total_cells = total_cells.saturating_add(area);
        if total_cells > 50_000 {
            return Err(Error::new("ワークブックが大きすぎて Markdown に変換できません"));
        }

        for r in minr..=maxr {
            out.push('|');
            let row_visible = row_vis(r);
            for c in minc..=maxc {
                if !row_visible || !col_vis(c) {
                    out.push_str("  |");
                    continue;
                }
                let cell = sheet.cell(r, c);
                let spill = sheet.spills.get(&(r, c));
                let raw = cell.map(|cell| cell.text.as_str()).unwrap_or("");
                let escaped = raw.replace('|', "\\|").replace('$', "\\$").replace('\n', "<br>");
                out.push(' ');
                let start = out.len();
                out.push_str(escaped.trim());
                record(&sheet_ref.name, r, c, cell, start, out.len());
                match cell.map(|cell| &cell.value) {
                    Some(ibis_doc_model::CellValue::Text(_))
                        if raw.trim_end().ends_with(XLSX_TEXT_CELL_MARKER)
                            || (spill.is_some() && raw.trim() == "#SPILL!")
                            || numeric_cell_marker(raw.trim()).is_some()
                            || ((raw.contains(XLSX_FORMULA_CELL_MARKER) || raw.contains(XLSX_FORMAT_CELL_MARKER)) && raw.trim_end().ends_with(XLSX_CELL_MARKER_END))
                            || !matches!(infer_markdown_cell_value(raw.trim()), ibis_doc_model::CellValue::Text(_)) =>
                    {
                        out.push(' ');
                        out.push_str(XLSX_TEXT_CELL_MARKER);
                    }
                    Some(ibis_doc_model::CellValue::Number(value))
                        if value.is_finite() && !matches!(
                            infer_markdown_cell_value(raw.trim()),
                            ibis_doc_model::CellValue::Number(inferred) if inferred.to_bits() == value.to_bits()
                        ) =>
                    {
                        use std::fmt::Write;
                        let _ = write!(out, " {XLSX_NUMBER_CELL_MARKER}{value}{XLSX_CELL_MARKER_END}");
                    }
                    _ => {}
                }
                if let Some(format) = cell.and_then(|cell| sheet.styles.get(cell.style as usize))
                    .map(|style| style.number_format.as_str())
                    .filter(|code| !code.is_empty() && !code.eq_ignore_ascii_case("General"))
                {
                    push_xlsx_string_marker(&mut out, XLSX_FORMAT_CELL_MARKER, format);
                }
                if let Some(formula) = cell.and_then(|cell| cell.formula.as_deref()) {
                    let formula = format!("={}", formula.strip_prefix('=').unwrap_or(formula));
                    if let Some(range) = spill {
                        let end = if cell.is_some_and(|cell| cell.value == ibis_doc_model::CellValue::Error(ibis_doc_model::CellError::Spill)) {
                            ibis_doc_model::cell_name(c, r)
                        } else {
                            ibis_doc_model::cell_name(range.last_column, range.last_row)
                        };
                        push_xlsx_json_marker(&mut out, XLSX_FORMULA_CELL_MARKER, &serde_json::json!({
                            "formula": formula,
                            "spill": end,
                        }));
                    } else if let Some(range) = sheet.fixed_arrays.get(&(r, c)) {
                        push_xlsx_json_marker(&mut out, XLSX_FORMULA_CELL_MARKER, &serde_json::json!({
                            "formula": formula,
                            "fixedArray": ibis_doc_model::cell_name(range.last_column, range.last_row),
                        }));
                    } else {
                        push_xlsx_string_marker(&mut out, XLSX_FORMULA_CELL_MARKER, &formula);
                    }
                }
                out.push_str(" |");
            }
            out.push('\n');
            if r == minr {
                out.push('|');
                for c in minc..=maxc {
                    let align = col_vis(c).then(|| sheet.cell(r, c)).flatten()
                        .and_then(|cell| sheet.styles.get(cell.style as usize))
                        .map(|st| match st.align {
                            CellAlign::Center | CellAlign::CenterAcross => " :---: |",
                            CellAlign::Right => " ---: |",
                            _ => " :--- |",
                        })
                        .unwrap_or(" :--- |");
                    out.push_str(align);
                }
                out.push('\n');
            }
        }
    }
    if has_hidden_sheet && !has_visible_sheet {
        return Err(Error::new("少なくとも1枚の表示シートが必要です"));
    }
    Ok(out)
}

// ============================================================================
// Parser implementation
// ============================================================================

struct ParseContext<'a> {
    next_id: u32,
    next_shape_id: u64,
    next_media_id: u64,
    next_comment_id: u32,
    styles: FlowStyles,
    bullet_num_id: Option<u32>,
    ordered_num_id: Option<u32>,
    footnote_ids: HashMap<String, u32>,
    footnote_order: Vec<(String, u32)>,
    next_footnote_id: u32,
    comments: Vec<Comment>,
    media: &'a mut Vec<Media>,
    source: Option<&'a crate::Source>,
    baseline_pres: Option<&'a Presentation>,
    base_dir: Option<&'a std::path::Path>,
}

impl<'a> ParseContext<'a> {
    fn fresh_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn fresh_shape_id(&mut self) -> ShapeId {
        let id = self.next_shape_id;
        self.next_shape_id += 1;
        ShapeId(id)
    }

    fn fresh_media_id(&mut self) -> MediaId {
        let id = self.next_media_id;
        self.next_media_id += 1;
        MediaId(id)
    }

    fn fresh_comment_id(&mut self) -> u32 {
        let id = self.next_comment_id;
        self.next_comment_id += 1;
        id
    }

    fn ensure_heading_style(&mut self, level: u8) -> String {
        let level = level.clamp(1, 6);
        let id = format!("Heading{level}");
        let name = format!("heading {level}");
        if let Some(existing) = self.styles.paragraph_style_named(&name) {
            return existing.to_string();
        }
        if self.styles.styles.contains_key(&id) {
            return id;
        }
        let size_pt = match level {
            1 => 20.0,
            2 => 16.0,
            3 => 13.5,
            4 => 12.0,
            5 => 11.0,
            _ => 10.5,
        };
        let before_pt = match level {
            1 => 14.0,
            2 => 12.0,
            3 => 10.0,
            _ => 6.0,
        };
        let after_pt = match level {
            1 | 2 => 6.0,
            _ => 4.0,
        };
        let size_hp = (size_pt * 100.0) as u32;
        let style = FlowStyle {
            name,
            kind: StyleKind::Paragraph,
            based_on: self.styles.default_paragraph.clone(),
            next: self.styles.default_paragraph.clone(),
            quick: true,
            ui_priority: Some(9),
            auto_update: false,
            conditional: Vec::new(),
            num_id: None,
            para: ParaFormat {
                before: Some((before_pt * EMU_PER_PT as f64) as Emu),
                after: Some((after_pt * EMU_PER_PT as f64) as Emu),
                keep_next: Some(true),
                keep_lines: Some(true),
                ..ParaFormat::default()
            },
            run: RunFormat {
                bold: Some(true),
                size: Some(size_hp),
                cs_size: Some(size_hp),
                ..RunFormat::default()
            },
        };
        self.styles.styles.insert(id.clone(), style);
        id
    }

    fn ensure_bullet_list(&mut self) -> u32 {
        if let Some(id) = self.bullet_num_id {
            return id;
        }
        let (numbering, num_id) = self.styles.numbering.with_new_list(true);
        self.styles.numbering = numbering;
        self.bullet_num_id = Some(num_id);
        num_id
    }

    fn ensure_ordered_list(&mut self) -> u32 {
        if let Some(id) = self.ordered_num_id {
            return id;
        }
        let (numbering, num_id) = self.styles.numbering.with_new_list(false);
        self.styles.numbering = numbering;
        self.ordered_num_id = Some(num_id);
        num_id
    }

    fn footnote_id_for(&mut self, label: &str) -> u32 {
        if let Some(&id) = self.footnote_ids.get(label) {
            return id;
        }
        let id = markdown_endnote_id(label).unwrap_or_else(|| {
            let id = self.next_footnote_id;
            self.next_footnote_id += 1;
            id
        });
        self.footnote_ids.insert(label.to_string(), id);
        self.footnote_order.push((label.to_string(), id));
        id
    }
}

fn parse_markdown_into(
    text: &str,
    doc: &mut FlowDocument,
    media: &mut Vec<Media>,
    source: Option<&crate::Source>,
    baseline_pres: Option<&Presentation>,
    base_dir: Option<&std::path::Path>,
) -> Result<(), Error> {
    let mut body_text = text;
    let mut frontmatter_text = None;

    let trimmed_start = text.trim_start_matches(['\n', '\r']);
    if let Some(after_open) = trimmed_start
        .strip_prefix("---\n")
        .or_else(|| trimmed_start.strip_prefix("---\r\n"))
    {
        if let Some((fm, rest)) = split_frontmatter_body(after_open) {
            frontmatter_text = Some(fm);
            body_text = rest;
        }
    }

    let mut ctx = ParseContext {
        next_id: doc.new_id().max(10),
        next_shape_id: doc.new_shape_id().max(10),
        next_media_id: media.iter().map(|m| m.id.0).max().unwrap_or(0) + 1,
        next_comment_id: 0,
        styles: (*doc.styles).clone(),
        bullet_num_id: None,
        ordered_num_id: None,
        footnote_ids: HashMap::new(),
        footnote_order: Vec::new(),
        next_footnote_id: 1,
        comments: Vec::new(),
        media,
        source,
        baseline_pres,
        base_dir,
    };

    if let Some(fm) = frontmatter_text {
        apply_frontmatter(fm, doc, &mut ctx);
    }

    while let Some((xml, rest)) = bibliography_fence(body_text) {
        doc.bibliography_sources.push(xml.to_owned());
        body_text = rest;
    }
    let mut footnote_defs: HashMap<String, String> = HashMap::new();
    let mut blocks = parse_blocks(body_text, &mut ctx, &mut footnote_defs)?;
    if let Some(Block::Paragraph(paragraph)) = blocks.last()
        && paragraph.section_break == ibis_doc_model::SectionBreak::None
        && let Some(section) = &paragraph.section
    {
        doc.page = PageSetup {
            size: section.page.size,
            top: section.page.top,
            right: section.page.right,
            bottom: section.page.bottom,
            left: section.page.left,
            header: section.page.header,
            footer: section.page.footer,
            gutter: section.page.gutter,
            columns: section.page.columns,
            first_page: section.page.first_page,
            ..doc.page
        };
        doc.restart_numbering = section.restart_numbering;
        doc.title_page = section.title_page;
        for (index, target) in [
            &mut doc.header, &mut doc.footer, &mut doc.header_first, &mut doc.footer_first,
            &mut doc.header_even, &mut doc.footer_even,
        ].into_iter().enumerate() {
            doc.linked[index] = section.parts[index].is_none();
            *target = section.parts[index].clone().unwrap_or_default();
        }
        blocks.pop();
    }
    if blocks.iter().any(|block| matches!(block, Block::Paragraph(p)
        if p.section.is_some() && p.section_break == ibis_doc_model::SectionBreak::None))
    {
        return Err(Error::new("最後のセクション設定は本文の末尾に置いてください"));
    }

    let first_existing_para_id = doc.body.iter().find_map(|b| match b {
        Block::Paragraph(p) => Some(p.id),
        _ => None,
    });

    doc.styles = Arc::new(ctx.styles.clone());
    doc.body = if blocks.is_empty() {
        let mut p = FlowParagraph {
            id: ctx.fresh_id(),
            style: doc.styles.default_paragraph.clone(),
            ..FlowParagraph::default()
        };
        p.resolve(&doc.styles);
        vec![Block::Paragraph(p)]
    } else {
        blocks
    };
    if let Some(existing_id) = first_existing_para_id {
        if let Some(first_para) = doc.body.iter_mut().find_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            _ => None,
        }) {
            first_para.id = existing_id;
        } else {
            let mut p = FlowParagraph {
                id: existing_id,
                style: doc.styles.default_paragraph.clone(),
                ..FlowParagraph::default()
            };
            p.resolve(&doc.styles);
            doc.body.push(Block::Paragraph(p));
        }
    }

    // Build notes in order of reference or definition.
    for (label, _def_text) in &footnote_defs {
        let _ = ctx.footnote_id_for(label);
    }
    let order = ctx.footnote_order.clone();
    for (label, note_id) in order {
        let endnote = markdown_endnote_id(&label).is_some();
        let content = footnote_defs
            .get(&label)
            .map(String::as_str)
            .unwrap_or(&label);
        let mut blocks = if let Some(markdown) = content.strip_prefix('\n') {
            // Note bodies use the same block grammar as the main story.
            parse_blocks(markdown, &mut ctx, &mut HashMap::new())?
        } else {
            let mut paragraph = FlowParagraph { id: ctx.fresh_id(), ..FlowParagraph::default() };
            paragraph.runs = parse_inlines(content, &mut ctx, &RunFormat::default());
            vec![Block::Paragraph(paragraph)]
        };
        let text_style = ctx
            .styles
            .paragraph_style_named(if endnote { "endnote text" } else { "footnote text" })
            .map(str::to_string)
            .or_else(|| ctx.styles.default_paragraph.clone());
        let mark_style = ctx
            .styles
            .styles
            .iter()
            .find(|(_, s)| {
                s.kind == StyleKind::Character
                    && s.name.eq_ignore_ascii_case(if endnote { "endnote reference" } else { "footnote reference" })
            })
            .map(|(id, _)| id.clone());
        let superscript = RunFormat {
            position: Some(VerticalPosition::Superscript),
            ..RunFormat::default()
        };
        let mark_format = if mark_style.is_some() {
            RunFormat::default()
        } else {
            superscript
        };
        if !matches!(blocks.first(), Some(Block::Paragraph(_))) {
            blocks.insert(0, Block::Paragraph(FlowParagraph {
                id: ctx.fresh_id(),
                ..FlowParagraph::default()
            }));
        }
        let Block::Paragraph(p) = &mut blocks[0] else { unreachable!() };
        if p.style.is_none() || p.style == ctx.styles.default_paragraph {
            p.style = text_style;
        }
        let mark = FlowRun {
            kind: FlowRunKind::NoteMark,
            style: FlowRunStyle::default(),
            format: mark_format,
            char_style: mark_style,
            link: None,
            revision: None,
            show_field_code: false,
            field_locked: false,
            control: None,
        };
        p.runs.splice(0..0, [mark, FlowRun::new(FlowRunKind::Text(" ".into()), FlowRunStyle::default())]);
        p.resolve(&ctx.styles);
        p.normalize();
        let notes = if endnote { &mut doc.endnotes } else { &mut doc.footnotes };
        notes.push(Note {
            id: note_id,
            blocks,
            ..Default::default()
        });
    }

    // Ensure watermark has a header part so saving to .docx succeeds.
    if doc.watermark.is_some() && doc.linked[0] {
        doc.linked[0] = false;
        if doc.header.is_empty() {
            let mut hp = FlowParagraph {
                id: ctx.fresh_id(),
                style: doc
                    .styles
                    .paragraph_style_named("header")
                    .map(str::to_string)
                    .or_else(|| doc.styles.default_paragraph.clone()),
                ..FlowParagraph::default()
            };
            hp.resolve(&doc.styles);
            doc.header.push(Block::Paragraph(hp));
        }
    }

    doc.comments = ctx.comments;

    // Resolve all paragraphs against final styles.
    doc.styles = Arc::new(ctx.styles);
    resolve_all_blocks(&mut doc.body, &doc.styles);
    for part in [
        &mut doc.header, &mut doc.footer, &mut doc.header_first, &mut doc.footer_first,
        &mut doc.header_even, &mut doc.footer_even,
    ] {
        resolve_all_blocks(part, &doc.styles);
    }

    // If a `[[toc]]` placeholder was inserted, populate its entries from the document headings.
    populate_toc_blocks(doc);

    for command in ibis_doc_model::flow_command::updated_fields(doc) {
        let _ = ibis_doc_model::flow_command::apply_to(doc, command);
    }

    Ok(())
}

fn resolve_all_blocks(blocks: &mut [Block], styles: &FlowStyles) {
    for block in blocks {
        match block {
            Block::Paragraph(p) => {
                p.resolve(styles);
                p.normalize();
            }
            Block::Table(t) => {
                t.apply_style(styles);
                for row in &mut t.rows {
                    for cell in &mut row.cells {
                        resolve_all_blocks(&mut cell.blocks, styles);
                    }
                }
            }
        }
    }
}

fn populate_toc_blocks(doc: &mut FlowDocument) {
    // Collect headings in body.
    let mut headings: Vec<(u8, String, String)> = Vec::new();
    let mut bm_counter = 1usize;
    for block in &mut doc.body {
        if let Block::Paragraph(p) = block {
            let level = p
                .style
                .as_deref()
                .and_then(|s| s.strip_prefix("Heading").or_else(|| s.strip_prefix("heading ")))
                .and_then(|n| n.trim().parse::<u8>().ok());
            if let Some(lvl) = level.filter(|&l| (1..=3).contains(&l)) {
                let text = p.text().trim().to_string();
                if !text.is_empty() {
                    let bm = if let Some(existing) = p.bookmarks.first() {
                        existing.clone()
                    } else {
                        let name = format!("_TocMd{bm_counter}");
                        bm_counter += 1;
                        p.bookmarks.push(name.clone());
                        name
                    };
                    headings.push((lvl, text, bm));
                }
            }
        }
    }
    if headings.is_empty() {
        return;
    }

    let mut idx = 0;
    while idx < doc.body.len() {
        let is_toc = matches!(
            &doc.body[idx],
            Block::Paragraph(p) if p.field_start.as_deref().is_some_and(|f| f.trim_start().starts_with("TOC")) && p.runs.is_empty()
        );
        if is_toc {
            let mut entries = Vec::new();
            let mut next_id = doc.new_id();
            for (i, (lvl, title, bm)) in headings.iter().enumerate() {
                let mut p = FlowParagraph {
                    id: next_id,
                    style: doc.styles.default_paragraph.clone(),
                    direct: ParaFormat {
                        left: Some(((lvl.saturating_sub(1) as i64) * 12) * EMU_PER_PT),
                        ..ParaFormat::default()
                    },
                    field_start: (i == 0).then(|| "TOC \\o \"1-3\" \\h \\z \\u".to_string()),
                    field_end: i + 1 == headings.len(),
                    ..FlowParagraph::default()
                };
                next_id += 1;
                let mut run = FlowRun::new(FlowRunKind::Text(title.clone()), FlowRunStyle::default());
                run.link = Some(Link::Anchor(bm.clone()));
                p.runs.push(run);
                p.resolve(&doc.styles);
                entries.push(Block::Paragraph(p));
            }
            let count = entries.len();
            doc.body.splice(idx..idx + 1, entries);
            idx += count;
        } else {
            idx += 1;
        }
    }
}

// ============================================================================
// Frontmatter parsing
// ============================================================================

fn apply_frontmatter(fm: &str, doc: &mut FlowDocument, ctx: &mut ParseContext<'_>) {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let mut current_section: Option<String> = None;

    for raw_line in fm.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw_line.chars().take_while(|&c| c == ' ' || c == '\t').count();
        if let Some((k, v)) = trimmed.split_once(':') {
            let key = k.trim().to_ascii_lowercase().replace('-', "_");
            let val = unquote_yaml(v.trim());
            if indent == 0 {
                if val.is_empty() {
                    current_section = Some(key);
                } else {
                    current_section = None;
                    if let Some(inner) = val.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                        for part in inner.split(',') {
                            if let Some((ik, iv)) = part.split_once(':') {
                                map.insert(
                                    format!("{key}.{}", ik.trim().to_ascii_lowercase().replace('-', "_")),
                                    unquote_yaml(iv.trim()),
                                );
                            }
                        }
                    }
                    map.insert(key, val);
                }
            } else if let Some(sec) = &current_section {
                map.insert(format!("{sec}.{key}"), val);
            }
        }
    }

    // Document properties
    if let Some(v) = map.get("title") {
        doc.properties.title = Some(v.clone());
    }
    if let Some(v) = map.get("subtitle").or_else(|| map.get("subject")) {
        doc.properties.subject = Some(v.clone());
    }
    if let Some(v) = map.get("author").or_else(|| map.get("creator")) {
        doc.properties.creator = Some(v.clone());
    }
    if let Some(v) = map.get("keywords").or_else(|| map.get("tags")) {
        let cleaned = v.trim_start_matches('[').trim_end_matches(']');
        let joined = cleaned
            .split(',')
            .map(|s| unquote_yaml(s.trim()))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        doc.properties.keywords = (!joined.is_empty()).then_some(joined);
    }
    if let Some(v) = map.get("description").or_else(|| map.get("summary")) {
        doc.properties.description = Some(v.clone());
    }
    if let Some(v) = map.get("category") {
        doc.properties.category = Some(v.clone());
    }
    if let Some(v) = map.get("company") {
        doc.properties.company = Some(v.clone());
    }

    // Page size & orientation
    if let Some(size_name) = map
        .get("size")
        .or_else(|| map.get("paper"))
        .or_else(|| map.get("page.size"))
        .or_else(|| map.get("page.paper"))
    {
        if let Some((w, h)) = parse_paper_size(size_name) {
            doc.page.size = EmuSize::new(w, h);
        }
    }
    if let Some(orient) = map
        .get("orientation")
        .or_else(|| map.get("page.orientation"))
    {
        let (w, h) = (
            doc.page.size.width.min(doc.page.size.height),
            doc.page.size.width.max(doc.page.size.height),
        );
        match orient.to_ascii_lowercase().as_str() {
            "landscape" | "横" => doc.page.size = EmuSize::new(h, w),
            "portrait" | "縦" => doc.page.size = EmuSize::new(w, h),
            _ => {}
        }
    }

    // Margins
    if let Some(margin_preset) = map.get("margins").or_else(|| map.get("page.margins")) {
        match margin_preset.to_ascii_lowercase().as_str() {
            "narrow" | "狭い" => {
                let m = (12.7 * EMU_PER_MM).round() as Emu;
                doc.page.top = m;
                doc.page.bottom = m;
                doc.page.left = m;
                doc.page.right = m;
            }
            "wide" | "広い" => {
                doc.page.top = (25.4 * EMU_PER_MM).round() as Emu;
                doc.page.bottom = (25.4 * EMU_PER_MM).round() as Emu;
                doc.page.left = (50.8 * EMU_PER_MM).round() as Emu;
                doc.page.right = (50.8 * EMU_PER_MM).round() as Emu;
            }
            "normal" | "標準" => {
                doc.page.top = (35.0 * EMU_PER_MM).round() as Emu;
                doc.page.bottom = (30.0 * EMU_PER_MM).round() as Emu;
                doc.page.left = (30.0 * EMU_PER_MM).round() as Emu;
                doc.page.right = (30.0 * EMU_PER_MM).round() as Emu;
            }
            other => {
                if let Some(emu) = parse_length_emu(other) {
                    doc.page.top = emu;
                    doc.page.bottom = emu;
                    doc.page.left = emu;
                    doc.page.right = emu;
                }
            }
        }
    }
    for (keys, field) in [
        (["margins.top", "page.margins.top", "margin_top"], &mut doc.page.top),
        (["margins.bottom", "page.margins.bottom", "margin_bottom"], &mut doc.page.bottom),
        (["margins.left", "page.margins.left", "margin_left"], &mut doc.page.left),
        (["margins.right", "page.margins.right", "margin_right"], &mut doc.page.right),
    ] {
        for k in keys {
            if let Some(v) = map.get(k).and_then(|s| parse_length_emu(s)) {
                *field = v;
            }
        }
    }

    if let Some(grid) = map.get("page_grid").and_then(|value| match value.as_str() {
        "default" => Some(ibis_doc_model::DocGridType::Default),
        "lines" => Some(ibis_doc_model::DocGridType::Lines),
        "linesAndChars" => Some(ibis_doc_model::DocGridType::LinesAndChars),
        "snapToChars" => Some(ibis_doc_model::DocGridType::SnapToChars),
        _ => None,
    }) {
        doc.page.doc_grid_type = grid;
        doc.page.line_pitch = map.get("page_line_pitch").and_then(|value| parse_length_emu(value));
        doc.page.char_space = map.get("page_char_space_4096pt")
            .and_then(|value| value.parse::<i32>().ok()).map(|units| i64::from(units) * 12700 / 4096);
    }

    // Columns
    if let Some(cols) = map
        .get("columns")
        .or_else(|| map.get("page.columns"))
        .and_then(|v| v.parse::<u32>().ok())
    {
        doc.page.columns.count = cols.clamp(1, 12) as u8;
    }
    if let Some(sep) = map
        .get("column_separator")
        .or_else(|| map.get("page.column_separator"))
    {
        doc.page.columns.separator = parse_bool(sep);
    }

    // Text direction (horizontal / vertical)
    if let Some(dir) = map
        .get("direction")
        .or_else(|| map.get("text_direction"))
        .or_else(|| map.get("page.direction"))
    {
        match dir.to_ascii_lowercase().as_str() {
            "vertical" | "tbrl" | "縦書き" => doc.page.direction = CellDirection::TbRl,
            "horizontal" | "lrtb" | "横書き" => doc.page.direction = CellDirection::LrTb,
            _ => {}
        }
    }

    // Line numbers
    if let Some(ln) = map.get("line_numbers").or_else(|| map.get("page.line_numbers")) {
        if parse_bool(ln) || ln.eq_ignore_ascii_case("continuous") || ln.eq_ignore_ascii_case("page") {
            let restart = if ln.eq_ignore_ascii_case("continuous") {
                LineNumberRestart::Continuous
            } else {
                LineNumberRestart::NewPage
            };
            doc.page.line_numbers = Some(LineNumbers {
                start: 1,
                count_by: 1,
                distance: 0,
                restart,
            });
        }
    }

    // Typography & Presets
    if let Some(font) = map.get("font").or_else(|| map.get("font_family")) {
        ctx.styles.defaults_run.font = Some(font.clone());
        ctx.styles.defaults_run.h_ansi_font = Some(font.clone());
        ctx.styles.defaults_run.east_asian_font = Some(font.clone());
        if let Some(normal_id) = ctx.styles.default_paragraph.clone() {
            if let Some(st) = ctx.styles.styles.get_mut(&normal_id) {
                st.run.font = Some(font.clone());
                st.run.h_ansi_font = Some(font.clone());
                st.run.east_asian_font = Some(font.clone());
            }
        }
    }
    if let Some(ea_font) = map.get("east_asian_font") {
        ctx.styles.defaults_run.east_asian_font = Some(ea_font.clone());
        if let Some(normal_id) = ctx.styles.default_paragraph.clone() {
            if let Some(st) = ctx.styles.styles.get_mut(&normal_id) {
                st.run.east_asian_font = Some(ea_font.clone());
            }
        }
    }
    if let Some(fs) = map.get("font_size") {
        let pt = fs.trim_end_matches("pt").trim().parse::<f64>().ok();
        if let Some(pt) = pt.filter(|&p| p >= 4.0 && p <= 144.0) {
            let hp = (pt * 100.0).round() as u32;
            ctx.styles.defaults_run.size = Some(hp);
            ctx.styles.defaults_run.cs_size = Some(hp);
            if let Some(normal_id) = ctx.styles.default_paragraph.clone() {
                if let Some(st) = ctx.styles.styles.get_mut(&normal_id) {
                    st.run.size = Some(hp);
                    st.run.cs_size = Some(hp);
                }
            }
        }
    }
    if let Some(preset_name) = map.get("style_set") {
        let preset = match preset_name.to_ascii_lowercase().replace('-', "_").as_str() {
            "default" | "既定" => Some(StyleSetPreset::Default),
            "casual" | "カジュアル" => Some(StyleSetPreset::Casual),
            "centered" | "中央揃え" => Some(StyleSetPreset::Centered),
            "formal" | "フォーマル" => Some(StyleSetPreset::Formal),
            "elegant" | "エレガント" => Some(StyleSetPreset::Elegant),
            "modern" | "モダン" => Some(StyleSetPreset::Modern),
            "simple" | "シンプル" => Some(StyleSetPreset::Simple),
            "minimalist" | "ミニマリスト" => Some(StyleSetPreset::Minimalist),
            _ => None,
        };
        if let Some(p) = preset {
            p.apply(&mut ctx.styles);
            doc.style_set_preset = Some(p);
        }
    }
    if let Some(spacing_name) = map.get("paragraph_spacing") {
        let preset = match spacing_name.to_ascii_lowercase().replace('-', "_").as_str() {
            "default" | "既定" => Some(ParagraphSpacingPreset::Default),
            "no_space" | "no_paragraph_space" | "none" | "段落間にスペースを入れない" => {
                Some(ParagraphSpacingPreset::NoSpace)
            }
            "compact" | "コンパクト" => Some(ParagraphSpacingPreset::Compact),
            "tight" | "狭い" => Some(ParagraphSpacingPreset::Tight),
            "open" | "開放的" => Some(ParagraphSpacingPreset::Open),
            "relaxed" | "やや広い" => Some(ParagraphSpacingPreset::Relaxed),
            "double" | "2倍" | "広い" => Some(ParagraphSpacingPreset::Double),
            _ => None,
        };
        if let Some(p) = preset {
            let (before, after, line) = p.spacing();
            if let Some(normal_id) = ctx.styles.default_paragraph.clone() {
                if let Some(st) = ctx.styles.styles.get_mut(&normal_id) {
                    st.para.before = Some(before);
                    st.para.after = Some(after);
                    st.para.line = Some(line);
                }
            }
            doc.paragraph_spacing_preset = Some(p);
        }
    }

    // Header & Footer
    if let Some(value) = map.get("even_and_odd_headers") {
        doc.even_and_odd = parse_bool(value);
    }
    if let Some(h) = map.get("header") {
        if !h.is_empty() {
            let p = build_header_footer_paragraph(h, false, ctx);
            doc.header = vec![Block::Paragraph(p)];
            doc.linked[0] = false;
        }
    }
    if let Some(f) = map.get("footer") {
        if !f.is_empty() {
            let p = build_header_footer_paragraph(f, true, ctx);
            doc.footer = vec![Block::Paragraph(p)];
            doc.linked[1] = false;
        }
    }

    // Watermark
    if let Some(wm) = map.get("watermark") {
        if !wm.is_empty() {
            doc.watermark = Some(Watermark {
                text: wm.clone(),
                font: ctx
                    .styles
                    .defaults_run
                    .east_asian_font
                    .clone()
                    .unwrap_or_else(|| "游明朝".to_string()),
                color: Rgb(0xC0C0C0),
                semitransparent: true,
                rotation: -45,
                size: None,
            });
        }
    }

    // Page color
    if let Some(pc) = map.get("page_color").and_then(|s| parse_hex_rgb(s)) {
        doc.page_color = Some(pc);
    }

    // Track changes
    if let Some(tc) = map.get("track_changes") {
        doc.track_changes = parse_bool(tc);
    }
}

fn build_header_footer_paragraph(
    template: &str,
    footer: bool,
    ctx: &mut ParseContext<'_>,
) -> FlowParagraph {
    let style_name = if footer { "footer" } else { "header" };
    let style = ctx
        .styles
        .paragraph_style_named(style_name)
        .map(str::to_string)
        .or_else(|| ctx.styles.default_paragraph.clone());
    let mut p = FlowParagraph {
        id: ctx.fresh_id(),
        style,
        direct: ParaFormat {
            align: if footer && template.trim() == "{page}" {
                Some(FlowAlign::Center)
            } else {
                None
            },
            ..ParaFormat::default()
        },
        ..FlowParagraph::default()
    };

    let mut rest = template;
    while !rest.is_empty() {
        let next_page = rest.find("{page}");
        let next_file = rest.find("{filename}");
        match (next_page, next_file) {
            (Some(pi), Some(fi)) if fi < pi => {
                if fi > 0 {
                    p.runs.extend(parse_inlines(&rest[..fi], ctx, &RunFormat::default()));
                }
                p.runs.push(FlowRun::new(
                    FlowRunKind::FileName("document.md".into()),
                    FlowRunStyle::default(),
                ));
                rest = &rest[fi + "{filename}".len()..];
            }
            (Some(pi), _) => {
                if pi > 0 {
                    p.runs.extend(parse_inlines(&rest[..pi], ctx, &RunFormat::default()));
                }
                p.runs.push(FlowRun::new(FlowRunKind::PageNumber, FlowRunStyle::default()));
                rest = &rest[pi + "{page}".len()..];
            }
            (None, Some(fi)) => {
                if fi > 0 {
                    p.runs.extend(parse_inlines(&rest[..fi], ctx, &RunFormat::default()));
                }
                p.runs.push(FlowRun::new(
                    FlowRunKind::FileName("document.md".into()),
                    FlowRunStyle::default(),
                ));
                rest = &rest[fi + "{filename}".len()..];
            }
            (None, None) => {
                p.runs.extend(parse_inlines(rest, ctx, &RunFormat::default()));
                break;
            }
        }
    }
    p
}

fn unquote_yaml(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        s[1..s.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}

fn parse_bool(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "on" | "1"
    )
}

fn parse_paper_size(name: &str) -> Option<(Emu, Emu)> {
    let mm = |w: f64, h: f64| ((w * EMU_PER_MM).round() as Emu, (h * EMU_PER_MM).round() as Emu);
    match name.trim().to_ascii_lowercase().as_str() {
        "a3" => Some(mm(297.0, 420.0)),
        "a4" => Some((11_906 * 635, 16_838 * 635)),
        "a5" => Some(mm(148.0, 210.0)),
        "b4" => Some(mm(257.0, 364.0)),
        "b5" => Some(mm(182.0, 257.0)),
        "letter" => Some((8 * EMU_PER_INCH + EMU_PER_INCH / 2, 11 * EMU_PER_INCH)),
        "legal" => Some((8 * EMU_PER_INCH + EMU_PER_INCH / 2, 14 * EMU_PER_INCH)),
        _ => None,
    }
}

fn parse_length_emu(s: &str) -> Option<Emu> {
    let s = s.trim();
    if let Some(v) = s.strip_suffix("mm") {
        v.trim()
            .parse::<f64>()
            .ok()
            .map(|n| (n * EMU_PER_MM).round() as Emu)
    } else if let Some(v) = s.strip_suffix("cm") {
        v.trim()
            .parse::<f64>()
            .ok()
            .map(|n| (n * 10.0 * EMU_PER_MM).round() as Emu)
    } else if let Some(v) = s.strip_suffix("in") {
        v.trim()
            .parse::<f64>()
            .ok()
            .map(|n| (n * EMU_PER_INCH as f64).round() as Emu)
    } else if let Some(v) = s.strip_suffix("pt") {
        v.trim()
            .parse::<f64>()
            .ok()
            .map(|n| (n * EMU_PER_PT as f64).round() as Emu)
    } else if let Some(v) = s.strip_suffix("px") {
        v.trim()
            .parse::<f64>()
            .ok()
            .map(|n| (n * 9_525.0).round() as Emu)
    } else {
        s.parse::<f64>()
            .ok()
            .map(|n| (n * EMU_PER_MM).round() as Emu)
    }
}

fn parse_hex_rgb(s: &str) -> Option<Rgb> {
    let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        u32::from_str_radix(hex, 16).ok().map(Rgb)
    } else {
        None
    }
}

// ============================================================================
// Block parsing
// ============================================================================

const SECTION_MARKER: &str = "::: section ";
const SECTION_PART_KEYS: [&str; 6] = [
    "header", "footer", "header_first", "footer_first", "header_even", "footer_even",
];

fn parse_section_marker(
    input: &str,
    ctx: &mut ParseContext<'_>,
) -> Result<(ibis_doc_model::SectionBreak, ibis_doc_model::SectionSetup), Error> {
    use ibis_doc_model::{PageColumns, SectionBreak, SectionSetup};
    let invalid = || Error::new("Markdown の section 指定が不正です");
    let data: serde_json::Value = serde_json::from_str(input).map_err(|_| invalid())?;
    let kind = match data["break"].as_str() {
        Some("end") => SectionBreak::None,
        Some("next-page") => SectionBreak::NextPage,
        Some("continuous") => SectionBreak::Continuous,
        Some("next-column") => SectionBreak::NextColumn,
        Some("odd-page") => SectionBreak::OddPage,
        Some("even-page") => SectionBreak::EvenPage,
        _ => return Err(invalid()),
    };
    let mut section = SectionSetup::default();
    let twips = |value: &serde_json::Value| value.as_i64()
        .filter(|value| (-31680..=31680).contains(value)).map(|value| value * 635);
    if let Some(size) = data.get("size") {
        let [width, height] = size.as_array().map(Vec::as_slice).ok_or_else(invalid)? else { return Err(invalid()); };
        let size = [width, height].map(|value| twips(value).filter(|value| *value > 0).ok_or_else(invalid));
        let [width, height] = size;
        section.page.size = EmuSize::new(width?, height?);
    }
    if let Some(margins) = data.get("margins") {
        let [top, right, bottom, left, header, footer, gutter] = margins.as_array().map(Vec::as_slice).ok_or_else(invalid)? else { return Err(invalid()); };
        section.page.top = twips(top).ok_or_else(invalid)?;
        section.page.right = twips(right).ok_or_else(invalid)?;
        section.page.bottom = twips(bottom).ok_or_else(invalid)?;
        section.page.left = twips(left).ok_or_else(invalid)?;
        section.page.header = twips(header).ok_or_else(invalid)?;
        section.page.footer = twips(footer).ok_or_else(invalid)?;
        section.page.gutter = twips(gutter).ok_or_else(invalid)?;
    }
    if let Some(count) = data.get("columns") {
        section.page.columns.count = count.as_u64().filter(|value| (1..=PageColumns::MAX as u64).contains(value))
            .ok_or_else(invalid)? as u8;
    }
    if let Some(gap) = data.get("column_gap") {
        section.page.columns.space = twips(gap).filter(|value| *value >= 0).ok_or_else(invalid)?;
    }
    if let Some(separator) = data.get("column_separator") {
        section.page.columns.separator = separator.as_bool().ok_or_else(invalid)?;
    }
    if let Some(first_page) = data.get("first_page") {
        section.page.first_page = first_page.as_u64().and_then(|value| u32::try_from(value).ok()).ok_or_else(invalid)?;
        section.restart_numbering = true;
    }
    if let Some(value) = data.get("title_page") {
        section.title_page = value.as_bool().ok_or_else(invalid)?;
    }
    for (kind, key) in ibis_doc_model::PartKind::ALL.into_iter().zip(SECTION_PART_KEYS) {
        if let Some(part) = data.get(key) {
            let text = part.as_str().ok_or_else(invalid)?;
            section.parts[kind.index()] = Some(vec![Block::Paragraph(build_header_footer_paragraph(text, kind.is_footer(), ctx))]);
        }
    }
    Ok((kind, section))
}

fn write_section_marker(
    out: &mut String,
    kind: ibis_doc_model::SectionBreak,
    page: &PageSetup,
    restart_numbering: bool,
    title_page: bool,
    parts: [Option<&[Block]>; 6],
) {
    use ibis_doc_model::SectionBreak;
    let kind = match kind {
        SectionBreak::None => "end",
        SectionBreak::NextPage => "next-page",
        SectionBreak::Continuous => "continuous",
        SectionBreak::NextColumn => "next-column",
        SectionBreak::OddPage => "odd-page",
        SectionBreak::EvenPage => "even-page",
    };
    let mut data = serde_json::json!({
        "break": kind,
        "size": [page.size.width / 635, page.size.height / 635],
        "margins": ([page.top, page.right, page.bottom, page.left, page.header, page.footer, page.gutter]
            .map(|value| value / 635)),
        "columns": page.columns.count,
        "column_gap": page.columns.space / 635,
        "column_separator": page.columns.separator,
    });
    if restart_numbering {
        data["first_page"] = page.first_page.into();
    }
    if title_page {
        data["title_page"] = true.into();
    }
    for (key, blocks) in SECTION_PART_KEYS.into_iter().zip(parts) {
        if let Some(blocks) = blocks {
            data[key] = serialize_header_footer(blocks).into();
        }
    }
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(SECTION_MARKER);
    out.push_str(&data.to_string());
    out.push_str("\n\n");
}

fn note_body_line(line: &str) -> Option<&str> {
    line.strip_prefix("    ").or_else(|| line.strip_prefix('\t'))
}

fn parse_blocks(
    body_text: &str,
    ctx: &mut ParseContext<'_>,
    footnote_defs: &mut HashMap<String, String>,
) -> Result<Vec<Block>, Error> {
    let lines: Vec<&str> = body_text.lines().collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Footnote definition: `[^id]: text...`
        if let Some(after_bracket) = trimmed.strip_prefix("[^") {
            if let Some((label, rest)) = after_bracket.split_once("]:") {
                if !label.is_empty() && !label.contains(' ') {
                    let mut def = rest.trim().to_string();
                    i += 1;
                    if def.is_empty() && lines.get(i).is_some_and(|line| note_body_line(line).is_some()) {
                        while let Some(raw) = lines.get(i) {
                            if let Some(line) = note_body_line(raw) {
                                def.push('\n');
                                def.push_str(line);
                                i += 1;
                            } else if raw.trim().is_empty() {
                                let start = i;
                                while lines.get(i).is_some_and(|line| line.trim().is_empty()) {
                                    i += 1;
                                }
                                if lines.get(i).and_then(|line| note_body_line(line)).is_none() {
                                    break;
                                }
                                def.extend(std::iter::repeat_n('\n', i - start));
                            } else {
                                break;
                            }
                        }
                    } else {
                        while let Some(next_raw) = lines.get(i).filter(|line| line.starts_with("  ") || line.starts_with('\t')) {
                            if !def.is_empty() {
                                def.push(' ');
                            }
                            def.push_str(next_raw.trim());
                            i += 1;
                        }
                    }
                    footnote_defs.insert(label.to_string(), def);
                    let _ = ctx.footnote_id_for(label);
                    continue;
                }
            }
        }

        if trimmed.ends_with(" -->")
            && (trimmed.starts_with("<!-- fixed-layout-page: ")
                || trimmed.starts_with("<!-- fixed-layout-diagnostic: "))
        {
            i += 1;
            continue;
        }

        if trimmed.starts_with(PPTX_SHAPE_MARKER) && trimmed.ends_with(" -->") {
            let paragraph = FlowParagraph {
                id: ctx.fresh_id(),
                runs: vec![FlowRun::new(FlowRunKind::Text(trimmed.to_owned()), FlowRunStyle::default())],
                ..Default::default()
            };
            blocks.push(Block::Paragraph(paragraph));
            i += 1;
            continue;
        }

        if let Some(input) = trimmed.strip_prefix(SECTION_MARKER) {
            let (kind, setup) = parse_section_marker(input, ctx)?;
            if kind != ibis_doc_model::SectionBreak::None
                && let Some(Block::Paragraph(previous)) = blocks.last_mut()
                && previous.section.is_none()
                && previous.field_start.is_none()
                && !previous.field_end
            {
                previous.section_break = kind;
                previous.section = Some(Arc::new(setup));
            } else {
                blocks.push(Block::Paragraph(FlowParagraph {
                    id: ctx.fresh_id(),
                    style: ctx.styles.default_paragraph.clone(),
                    section_break: kind,
                    section: Some(Arc::new(setup)),
                    ..FlowParagraph::default()
                }));
            }
            i += 1;
            continue;
        }

        // Page break & Column break & TOC directives
        if matches!(
            trimmed,
            "\\newpage" | "\\pagebreak" | "::: pagebreak" | "<!-- pagebreak -->"
        ) {
            let mut p = FlowParagraph {
                id: ctx.fresh_id(),
                style: ctx.styles.default_paragraph.clone(),
                runs: vec![FlowRun::new(FlowRunKind::PageBreak, FlowRunStyle::default())],
                ..FlowParagraph::default()
            };
            p.resolve(&ctx.styles);
            blocks.push(Block::Paragraph(p));
            i += 1;
            continue;
        }
        if matches!(
            trimmed,
            "\\columnbreak" | "::: columnbreak" | "<!-- columnbreak -->"
        ) {
            let mut p = FlowParagraph {
                id: ctx.fresh_id(),
                style: ctx.styles.default_paragraph.clone(),
                runs: vec![FlowRun::new(
                    FlowRunKind::ColumnBreak,
                    FlowRunStyle::default(),
                )],
                ..FlowParagraph::default()
            };
            p.resolve(&ctx.styles);
            blocks.push(Block::Paragraph(p));
            i += 1;
            continue;
        }
        let field = if matches!(trimmed, "[[toc]]" | "[TOC]" | "::: toc") {
            Some(r#"TOC \o "1-3" \h \z \u"#.to_owned())
        } else {
            markdown_bibliography_field(trimmed)
        };
        if let Some(instruction) = field {
            let p = FlowParagraph {
                id: ctx.fresh_id(),
                style: ctx.styles.default_paragraph.clone(),
                field_start: Some(instruction),
                field_end: true,
                ..FlowParagraph::default()
            };
            blocks.push(Block::Paragraph(p));
            i += 1;
            continue;
        }

        // Horizontal rule: `---`, `***`, `___`
        if is_horizontal_rule(trimmed) {
            let border = FlowBorder {
                width: 635 * 6,
                color: Rgb(0xBFBFBF),
                style: BorderStyle::Single,
                space: 12_700,
                art: None,
            };
            let mut p = FlowParagraph {
                id: ctx.fresh_id(),
                style: ctx.styles.default_paragraph.clone(),
                direct: ParaFormat {
                    borders: [None, None, Some(Some(border)), None],
                    after: Some(6 * EMU_PER_PT),
                    ..ParaFormat::default()
                },
                ..FlowParagraph::default()
            };
            p.resolve(&ctx.styles);
            blocks.push(Block::Paragraph(p));
            i += 1;
            continue;
        }

        // Fenced code / math / chart block: ```lang ... ```
        if let Some((fence_marker, fence_info)) = opening_fence(trimmed) {
            let lang = fence_info.trim().to_ascii_lowercase();
            i += 1;
            let mut content_lines = Vec::new();
            while i < lines.len() {
                if closes_fence(lines[i].trim(), fence_marker) {
                    i += 1;
                    break;
                }
                content_lines.push(lines[i]);
                i += 1;
            }
            let content = content_lines.join("\n");
            match lang.split_whitespace().next().unwrap_or("") {
                "math" | "latex" | "tex" => {
                    blocks.push(build_display_math_block(&content, None, ctx));
                }
                "chart" => {
                    blocks.push(build_chart_block(&content, ctx)?);
                }
                "mermaid" => {
                    blocks.push(build_mermaid_block(&content, ctx)?);
                }
                _ => {
                    blocks.push(build_code_block(&content, ctx));
                }
            }
            continue;
        }

        // Display math block: $$ ... $$
        if trimmed.starts_with("$$") {
            if let Some((inner, bookmark)) = display_math_close(&trimmed[2..])
                && !inner.trim().is_empty()
            {
                blocks.push(build_display_math_block(inner, bookmark, ctx));
                i += 1;
                continue;
            }
            let mut math_lines = Vec::new();
            let mut bookmark = None;
            let first_after = trimmed.strip_prefix("$$").unwrap_or("").trim();
            if !first_after.is_empty() {
                math_lines.push(first_after.to_string());
            }
            i += 1;
            while i < lines.len() {
                let l = lines[i].trim();
                if let Some((before_close, id)) = display_math_close(l) {
                    bookmark = id;
                    if !before_close.trim().is_empty() {
                        math_lines.push(before_close.trim().to_string());
                    }
                    i += 1;
                    break;
                }
                math_lines.push(lines[i].to_string());
                i += 1;
            }
            blocks.push(build_display_math_block(&math_lines.join("\n"), bookmark, ctx));
            continue;
        }

        // ATX Heading: `#` .. `######`
        if let Some((level, heading_text)) = parse_atx_heading(trimmed) {
            let (clean_text, attrs) = strip_trailing_block_attrs(heading_text);
            let style_id = ctx.ensure_heading_style(level);
            let mut p = FlowParagraph {
                id: ctx.fresh_id(),
                style: Some(style_id),
                direct: attrs.paragraph_format(),
                bookmarks: attrs.id.into_iter().collect(),
                runs: parse_inlines(clean_text, ctx, &RunFormat::default()),
                ..FlowParagraph::default()
            };
            p.resolve(&ctx.styles);
            p.normalize();
            blocks.push(Block::Paragraph(p));
            i += 1;
            continue;
        }

        // Pipe table: row followed by separator row `| --- | --- |`
        if is_table_row(trimmed) && i + 1 < lines.len() && is_table_separator(lines[i + 1].trim()) {
            let header_cells = split_table_row(trimmed);
            let alignments = parse_table_alignments(lines[i + 1].trim());
            i += 2;
            let mut body_rows = Vec::new();
            while i < lines.len() && is_table_row(lines[i].trim()) {
                body_rows.push(split_table_row(lines[i].trim()));
                i += 1;
            }
            blocks.push(build_table_block(header_cells, alignments, body_rows, ctx, 0));
            continue;
        }

        // Blockquote or GitHub Alert: `> ...`
        if trimmed.starts_with('>') {
            let mut quote_lines = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let l = lines[i].trim_start();
                let stripped = l.strip_prefix("> ").or_else(|| l.strip_prefix('>')).unwrap_or("");
                quote_lines.push(stripped);
                i += 1;
            }
            blocks.extend(build_blockquote_blocks(&quote_lines, ctx));
            continue;
        }

        // Bullet / Ordered / Task List item
        if let Some(item) = parse_list_marker(raw) {
            let num_id = if item.ordered {
                ctx.ensure_ordered_list()
            } else {
                ctx.ensure_bullet_list()
            };
            let (clean_text, attrs) = strip_trailing_block_attrs(item.text);
            let mut runs = Vec::new();
            if let Some(checked) = item.task_checked {
                runs.push(FlowRun::new(
                    FlowRunKind::CheckBox {
                        checked,
                        checked_char: '☑',
                        unchecked_char: '☐',
                    },
                    FlowRunStyle::default(),
                ));
                runs.push(FlowRun::new(
                    FlowRunKind::Text(" ".into()),
                    FlowRunStyle::default(),
                ));
            }
            runs.extend(parse_inlines(clean_text, ctx, &RunFormat::default()));
            let mut p = FlowParagraph {
                id: ctx.fresh_id(),
                style: ctx.styles.default_paragraph.clone(),
                direct: ParaFormat {
                    align: attrs.align,
                    num_id: if item.task_checked.is_some() { None } else { Some(num_id) },
                    level: if item.task_checked.is_some() { None } else { Some(item.level as u32) },
                    ..ParaFormat::default()
                },
                bookmarks: attrs.id.into_iter().collect(),
                list: if item.task_checked.is_some() {
                    None
                } else {
                    Some(ListRef {
                        num_id,
                        level: item.level as u32,
                    })
                },
                runs,
                ..FlowParagraph::default()
            };
            if item.task_checked.is_some() && item.level > 0 {
                p.direct.left = Some((item.level as i64) * 18 * EMU_PER_PT);
            }
            p.resolve(&ctx.styles);
            p.normalize();
            blocks.push(Block::Paragraph(p));
            i += 1;
            continue;
        }

        // Normal paragraph (accumulate consecutive non-blank, non-special lines)
        let mut para_lines: Vec<&str> = Vec::new();
        while i < lines.len() {
            let l_raw = lines[i];
            let l_trim = l_raw.trim();
            if l_trim.is_empty()
                || l_trim.starts_with("```")
                || l_trim.starts_with("~~~")
                || l_trim.starts_with("$$")
                || l_trim.starts_with('>')
                || l_trim.starts_with(":::")
                || l_trim == "\\newpage"
                || l_trim == "\\pagebreak"
                || l_trim == "\\columnbreak"
                || l_trim == "[[toc]]"
                || l_trim == "[TOC]"
                || markdown_bibliography_field(l_trim).is_some()
                || is_horizontal_rule(l_trim)
                || parse_atx_heading(l_trim).is_some()
                || parse_list_marker(l_raw).is_some()
                || (is_table_row(l_trim)
                    && i + 1 < lines.len()
                    && is_table_separator(lines[i + 1].trim()))
                || (l_trim.starts_with("[^") && l_trim.contains("]:"))
            {
                break;
            }
            para_lines.push(l_raw);
            i += 1;
        }

        if !para_lines.is_empty() {
            let mut runs = Vec::new();
            let mut para_attrs = BlockAttrs::default();
            for (li, pline) in para_lines.iter().enumerate() {
                let hard_break = pline.ends_with("  ") || pline.ends_with('\\');
                let mut content = pline.trim_end();
                if content.ends_with('\\') {
                    content = &content[..content.len() - 1];
                }
                let content = content.trim();
                let content = if li + 1 == para_lines.len() {
                    let (c, a) = strip_trailing_block_attrs(content);
                    para_attrs = a;
                    c
                } else {
                    content
                };
                if li > 0 && !runs.is_empty() {
                    let prev_hard = para_lines[li - 1].ends_with("  ")
                        || para_lines[li - 1].trim_end().ends_with('\\');
                    if prev_hard {
                        runs.push(FlowRun::new(
                            FlowRunKind::LineBreak,
                            FlowRunStyle::default(),
                        ));
                    } else {
                        runs.push(FlowRun::new(
                            FlowRunKind::Text(" ".into()),
                            FlowRunStyle::default(),
                        ));
                    }
                }
                let _ = hard_break;
                runs.extend(parse_inlines(content, ctx, &RunFormat::default()));
            }
            let mut p = FlowParagraph {
                id: ctx.fresh_id(),
                style: ctx.styles.default_paragraph.clone(),
                direct: para_attrs.paragraph_format(),
                bookmarks: para_attrs.id.into_iter().collect(),
                runs,
                ..FlowParagraph::default()
            };
            p.resolve(&ctx.styles);
            p.normalize();
            blocks.push(Block::Paragraph(p));
        }
    }

    Ok(blocks)
}

#[derive(Default)]
struct BlockAttrs {
    id: Option<String>,
    align: Option<FlowAlign>,
    spacing: Option<(Emu, Emu, LineHeight)>,
}

impl BlockAttrs {
    fn paragraph_format(&self) -> ParaFormat {
        let mut format = ParaFormat { align: self.align, ..Default::default() };
        if let Some((before, after, line)) = self.spacing {
            format.before = Some(before);
            format.after = Some(after);
            format.line = Some(line);
        }
        format
    }
}

fn strip_trailing_block_attrs(text: &str) -> (&str, BlockAttrs) {
    let trimmed = text.trim_end();
    if let Some(open) = trimmed.rfind('{') {
        if trimmed.ends_with('}') && open > 0 {
            // Do not mistake CriticMarkup `{++...++}` or ruby `{漢字|かな}` for block attributes.
            let inner = &trimmed[open + 1..trimmed.len() - 1];
            if !inner.contains('|')
                && !inner.starts_with('+')
                && !inner.starts_with('-')
                && !inner.starts_with('~')
                && !inner.starts_with('>')
                && !inner.starts_with('=')
            {
                let mut attrs = BlockAttrs::default();
                let mut recognized = false;
                for token in inner.split_whitespace() {
                    if let Some(id) = token.strip_prefix('#') {
                        if !id.is_empty() {
                            attrs.id = Some(id.to_string());
                            recognized = true;
                        }
                    } else if let Some(al) = token.strip_prefix("align=").or_else(|| token.strip_prefix("text-align=")) {
                        attrs.align = match al.trim_matches('"').to_ascii_lowercase().as_str() {
                            "center" => Some(FlowAlign::Center),
                            "right" => Some(FlowAlign::Right),
                            "justify" | "both" => Some(FlowAlign::Justify),
                            "distribute" => Some(FlowAlign::Distribute),
                            "left" => Some(FlowAlign::Left),
                            _ => None,
                        };
                        if attrs.align.is_some() {
                            recognized = true;
                        }
                    } else if let Some(value) = token.strip_prefix("docx-spacing=") {
                        let parts: Vec<_> = value.split(';').collect();
                        if let [before, after, line] = parts.as_slice() {
                            attrs.spacing = form_spacing(before, after, line);
                            recognized |= attrs.spacing.is_some();
                        }
                    } else if matches!(token, ".center" | ".right" | ".justify" | ".distribute") {
                        attrs.align = match token {
                            ".center" => Some(FlowAlign::Center),
                            ".right" => Some(FlowAlign::Right),
                            ".justify" => Some(FlowAlign::Justify),
                            ".distribute" => Some(FlowAlign::Distribute),
                            _ => None,
                        };
                        recognized = true;
                    }
                }
                if recognized {
                    return (trimmed[..open].trim_end(), attrs);
                }
            }
        }
    }
    (trimmed, BlockAttrs::default())
}

fn is_horizontal_rule(s: &str) -> bool {
    if s.len() < 3 {
        return false;
    }
    let chars: Vec<char> = s.chars().filter(|&c| c != ' ').collect();
    if chars.len() < 3 {
        return false;
    }
    let first = chars[0];
    matches!(first, '-' | '*' | '_') && chars.iter().all(|&c| c == first)
}

fn parse_atx_heading(s: &str) -> Option<(u8, &str)> {
    let hashes = s.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = &s[hashes..];
        if rest.is_empty() {
            return Some((hashes as u8, ""));
        }
        if let Some(stripped) = rest.strip_prefix(' ') {
            let trimmed = stripped.trim().trim_end_matches('#').trim();
            return Some((hashes as u8, trimmed));
        }
    }
    None
}

struct ListMarker<'a> {
    ordered: bool,
    level: u8,
    task_checked: Option<bool>,
    text: &'a str,
}

fn parse_list_marker(raw_line: &str) -> Option<ListMarker<'_>> {
    let mut indent_spaces = 0usize;
    let mut byte_idx = 0usize;
    for ch in raw_line.chars() {
        match ch {
            ' ' => {
                indent_spaces += 1;
                byte_idx += 1;
            }
            '\t' => {
                indent_spaces += 2;
                byte_idx += 1;
            }
            _ => break,
        }
    }
    let level = ((indent_spaces / 2) as u8).min(8);
    let rest = &raw_line[byte_idx..];

    // Unordered: `- `, `* `, `+ `
    if let Some(after_bullet) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        if let Some(task_rest) = after_bullet.strip_prefix("[ ] ") {
            return Some(ListMarker {
                ordered: false,
                level,
                task_checked: Some(false),
                text: task_rest,
            });
        }
        if let Some(task_rest) = after_bullet
            .strip_prefix("[x] ")
            .or_else(|| after_bullet.strip_prefix("[X] "))
        {
            return Some(ListMarker {
                ordered: false,
                level,
                task_checked: Some(true),
                text: task_rest,
            });
        }
        return Some(ListMarker {
            ordered: false,
            level,
            task_checked: None,
            text: after_bullet,
        });
    }

    // Ordered: `1. `, `2. `, etc.
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let after_digits = &rest[digits..];
        if let Some(after_dot) = after_digits
            .strip_prefix(". ")
            .or_else(|| after_digits.strip_prefix(") "))
        {
            return Some(ListMarker {
                ordered: true,
                level,
                task_checked: None,
                text: after_dot,
            });
        }
    }

    None
}

fn build_code_block(code: &str, ctx: &mut ParseContext<'_>) -> Block {
    let border = FlowBorder {
        width: 635 * 4,
        color: Rgb(0xD9D9D9),
        style: BorderStyle::Single,
        space: 4 * EMU_PER_PT,
        art: None,
    };
    let mono_format = RunFormat {
        font: Some("Consolas".to_string()),
        h_ansi_font: Some("Consolas".to_string()),
        east_asian_font: Some("ＭＳ ゴシック".to_string()),
        size: Some(950),
        cs_size: Some(950),
        no_proof: Some(true),
        ..RunFormat::default()
    };
    let mut runs = Vec::new();
    for (idx, line) in code.split('\n').enumerate() {
        if idx > 0 {
            runs.push(FlowRun {
                kind: FlowRunKind::LineBreak,
                style: FlowRunStyle::default(),
                format: mono_format.clone(),
                char_style: None,
                link: None,
                revision: None,
                show_field_code: false,
                field_locked: false,
                control: None,
            });
        }
        let line_clean = line.strip_suffix('\r').unwrap_or(line);
        if !line_clean.is_empty() {
            runs.push(FlowRun {
                kind: FlowRunKind::Text(line_clean.to_string()),
                style: FlowRunStyle::default(),
                format: mono_format.clone(),
                char_style: None,
                link: None,
                revision: None,
                show_field_code: false,
                field_locked: false,
                control: None,
            });
        }
    }
    let mut p = FlowParagraph {
        id: ctx.fresh_id(),
        style: ctx.styles.default_paragraph.clone(),
        direct: ParaFormat {
            shading: Some(Some(Rgb(0xF6F8FA))),
            borders: [
                Some(Some(border)),
                Some(Some(border)),
                Some(Some(border)),
                Some(Some(border)),
            ],
            left: Some(8 * EMU_PER_PT),
            right: Some(8 * EMU_PER_PT),
            before: Some(6 * EMU_PER_PT),
            after: Some(6 * EMU_PER_PT),
            line: Some(LineHeight::Auto(276)),
            ..ParaFormat::default()
        },
        runs,
        ..FlowParagraph::default()
    };
    p.resolve(&ctx.styles);
    Block::Paragraph(p)
}

/// Split a closing math delimiter from an optional Word-compatible bookmark.
fn display_math_close(line: &str) -> Option<(&str, Option<&str>)> {
    if let Some(math) = line.strip_suffix("$$") {
        return Some((math, None));
    }
    let (math, label) = line.strip_suffix('}')?.rsplit_once("{#")?;
    let math = math.trim_end().strip_suffix("$$")?;
    is_equation_bookmark(label).then_some((math, Some(label)))
}

fn is_equation_bookmark(label: &str) -> bool {
    label.len() <= 40
        && label.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn build_display_math_block(latex: &str, bookmark: Option<&str>, ctx: &mut ParseContext<'_>) -> Block {
    let math = parse_latex(latex.trim());
    let mut p = FlowParagraph {
        id: ctx.fresh_id(),
        style: ctx.styles.default_paragraph.clone(),
        direct: ParaFormat {
            align: Some(FlowAlign::Center),
            before: Some(6 * EMU_PER_PT),
            after: Some(6 * EMU_PER_PT),
            ..ParaFormat::default()
        },
        bookmarks: bookmark.map(str::to_owned).into_iter().collect(),
        runs: vec![FlowRun::new(
            FlowRunKind::Math {
                math,
                display: true,
            },
            FlowRunStyle::default(),
        )],
        ..FlowParagraph::default()
    };
    p.resolve(&ctx.styles);
    Block::Paragraph(p)
}

fn build_blockquote_blocks(lines: &[&str], ctx: &mut ParseContext<'_>) -> Vec<Block> {
    let first = lines.first().map(|s| s.trim()).unwrap_or("");
    let alert = match first {
        "[!NOTE]" => Some(("NOTE", Rgb(0x0969DA), Rgb(0xEDF5FF))),
        "[!TIP]" => Some(("TIP", Rgb(0x1A7F37), Rgb(0xEEFAF2))),
        "[!IMPORTANT]" => Some(("IMPORTANT", Rgb(0x8250DF), Rgb(0xF6F0FF))),
        "[!WARNING]" => Some(("WARNING", Rgb(0x9A6700), Rgb(0xFFF8E6))),
        "[!CAUTION]" => Some(("CAUTION", Rgb(0xCF222E), Rgb(0xFFECEB))),
        _ => None,
    };

    let (accent_color, bg_color, content_lines) = match alert {
        Some((_, accent, bg)) => (accent, Some(bg), &lines[1..]),
        None => (Rgb(0xD0D7DE), None, lines),
    };

    let left_border = FlowBorder {
        width: 635 * 18,
        color: accent_color,
        style: BorderStyle::Single,
        space: 8 * EMU_PER_PT,
        art: None,
    };
    let para_direct = ParaFormat {
        left: Some(14 * EMU_PER_PT),
        before: Some(4 * EMU_PER_PT),
        after: Some(4 * EMU_PER_PT),
        shading: bg_color.map(Some),
        borders: [None, None, None, Some(Some(left_border))],
        ..ParaFormat::default()
    };

    let mut out = Vec::new();
    if let Some((label, accent, _)) = alert {
        let mut runs = vec![FlowRun {
            kind: FlowRunKind::Text(label.to_string()),
            style: FlowRunStyle::default(),
            format: RunFormat {
                bold: Some(true),
                color: Some(accent),
                ..RunFormat::default()
            },
            char_style: None,
            link: None,
            revision: None,
            show_field_code: false,
            field_locked: false,
            control: None,
        }];
        let joined = content_lines
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !joined.is_empty() {
            runs.push(FlowRun::new(
                FlowRunKind::LineBreak,
                FlowRunStyle::default(),
            ));
            runs.extend(parse_inlines(&joined, ctx, &RunFormat::default()));
        }
        let mut p = FlowParagraph {
            id: ctx.fresh_id(),
            style: ctx.styles.default_paragraph.clone(),
            direct: para_direct,
            runs,
            ..FlowParagraph::default()
        };
        p.resolve(&ctx.styles);
        p.normalize();
        out.push(Block::Paragraph(p));
        return out;
    }

    let joined = content_lines
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut p = FlowParagraph {
        id: ctx.fresh_id(),
        style: ctx.styles.default_paragraph.clone(),
        direct: para_direct,
        runs: parse_inlines(&joined, ctx, &RunFormat::default()),
        ..FlowParagraph::default()
    };
    p.resolve(&ctx.styles);
    p.normalize();
    out.push(Block::Paragraph(p));
    out
}

// ============================================================================
// Pipe Table parsing
// ============================================================================

fn is_table_row(s: &str) -> bool {
    let s = s.trim();
    s.starts_with('|') && s.ends_with('|') && s.len() >= 3
}

fn is_table_separator(s: &str) -> bool {
    if !is_table_row(s) {
        return false;
    }
    let cells = split_table_row(s);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let t = c.trim().trim_start_matches(':').trim_end_matches(':');
            t.len() >= 3 && t.chars().all(|ch| ch == '-')
        })
}

fn split_table_row(s: &str) -> Vec<String> {
    let inner = s
        .trim()
        .strip_prefix('|')
        .unwrap_or(s)
        .strip_suffix('|')
        .unwrap_or(s);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'|') {
            cur.push('|');
            chars.next();
        } else if ch == '|' {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn parse_table_alignments(sep: &str) -> Vec<Option<FlowAlign>> {
    split_table_row(sep)
        .into_iter()
        .map(|c| {
            let t = c.trim();
            match (t.starts_with(':'), t.ends_with(':')) {
                (true, true) => Some(FlowAlign::Center),
                (false, true) => Some(FlowAlign::Right),
                (true, false) => Some(FlowAlign::Left),
                (false, false) => None,
            }
        })
        .collect()
}

fn table_colspan<'a>(cell: &'a str, row: &[String], column: usize) -> (&'a str, usize) {
    if let Some((text, marker)) = cell.strip_suffix(" -->").and_then(|cell| cell.rsplit_once("<!-- colspan:"))
        && let Ok(span) = marker.parse::<u32>()
        && span > 1
        && let Some(end) = column.checked_add(span as usize)
        && row.get(column + 1..end).is_some_and(|covered| covered.iter().all(String::is_empty))
    {
        return (text.trim_end(), span as usize);
    }
    (cell, 1)
}

const NESTED_TABLE_START: &str = "<!-- nested-table: ";
const NESTED_TABLE_END: &str = " -->";

fn parse_nested_table(payload: &str, ctx: &mut ParseContext<'_>, depth: usize) -> Option<Block> {
    let decode_line = |line: &str| line.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
    let rows: Vec<_> = payload.split("<br>").map(decode_line).collect();
    let mut lines = rows.iter().map(String::as_str);
    let header = lines.next()?;
    let separator = lines.next()?;
    if !is_table_row(header) || !is_table_separator(separator) || !lines.clone().all(is_table_row) {
        return None;
    }
    Some(build_table_block(
        split_table_row(header),
        parse_table_alignments(separator),
        lines.map(split_table_row).collect(),
        ctx,
        depth,
    ))
}

const PPTX_NO_TABLE_HEADER: &str = "<!-- pptx-no-header -->";

/// Optional table geometry attached to the first cell of a form table.
struct FormTableLayout {
    columns: Vec<Emu>,
    align: FlowAlign,
    anchor: ibis_doc_model::CellAnchor,
    first_row_header: bool,
}

fn take_table_comment(cell: &mut String, prefix: &str) -> Option<String> {
    let (value, rest) = cell.strip_prefix(prefix)?.split_once(" -->")?;
    let value = value.to_owned();
    *cell = rest.trim_start().to_owned();
    Some(value)
}

fn form_table_layout(cell: &mut String, column_count: usize) -> Option<FormTableLayout> {
    let comment = take_table_comment(cell, "<!-- docx-table:")?;
    let [widths, align, anchor, header] = comment.split(';').collect::<Vec<_>>().try_into().ok()?;
    let columns = widths.split(',').map(|value: &str| {
        value.parse::<i64>().ok().filter(|twips| (1..=31680).contains(twips)).map(|twips| twips * 635)
    }).collect::<Option<Vec<_>>>()?;
    if columns.len() != column_count {
        return None;
    }
    Some(FormTableLayout {
        columns,
        align: match align { "left" => FlowAlign::Left, "center" => FlowAlign::Center, "right" => FlowAlign::Right, _ => return None },
        anchor: match anchor {
            "top" => ibis_doc_model::CellAnchor::Top,
            "center" => ibis_doc_model::CellAnchor::Center,
            "bottom" => ibis_doc_model::CellAnchor::Bottom,
            _ => return None,
        },
        first_row_header: header.parse().ok()?,
    })
}

fn form_row_height(cell: &mut String) -> RowHeight {
    let Some(comment) = take_table_comment(cell, "<!-- docx-row:") else { return RowHeight::Auto };
    let Some((rule, height)) = comment.split_once('=') else { return RowHeight::Auto };
    let Ok(height) = height.parse::<i64>() else { return RowHeight::Auto };
    if !(1..=31680).contains(&height) {
        return RowHeight::Auto;
    }
    match rule {
        "min" => RowHeight::AtLeast(height * 635),
        "exact" => RowHeight::Exact(height * 635),
        _ => RowHeight::Auto,
    }
}

const FORM_PARAGRAPH: &str = "<!-- docx-p:";
const FORM_RUN_SIZE: &str = "<!-- docx-size:";
const FORM_RUN_END: &str = "<!-- /docx-size -->";

fn form_paragraph_format(body: &str, fallback: Option<FlowAlign>) -> (&str, ParaFormat, RunFormat) {
    let mut paragraph = ParaFormat { align: fallback, ..Default::default() };
    let mut mark = RunFormat::default();
    let Some((properties, text)) = body.strip_prefix(FORM_PARAGRAPH).and_then(|s| s.split_once(" -->")) else {
        return (body, paragraph, mark);
    };
    let parts: Vec<_> = properties.split(';').collect();
    if let [align, before, after, line_spacing, size] = parts.as_slice() {
        paragraph.align = match *align {
            "left" => Some(FlowAlign::Left), "center" => Some(FlowAlign::Center), "right" => Some(FlowAlign::Right),
            "justify" => Some(FlowAlign::Justify), "distribute" => Some(FlowAlign::Distribute), _ => fallback,
        };
        if let Some((before, after, line)) = form_spacing(before, after, line_spacing) {
            paragraph.before = Some(before);
            paragraph.after = Some(after);
            paragraph.line = Some(line);
        }
        mark.size = size.parse().ok().filter(|n| (1..=14400).contains(n));
    }
    (text, paragraph, mark)
}

fn form_spacing(before: &str, after: &str, line_spacing: &str) -> Option<(Emu, Emu, LineHeight)> {
    let twips = |value: &str| value.parse::<i64>().ok().filter(|n| (0..=31680).contains(n)).map(|n| n * 635);
    let (rule, value) = line_spacing.split_once('=')?;
    let line = match rule {
        "auto" => value.parse().ok().filter(|n| (1..=31680).contains(n)).map(LineHeight::Auto),
        "min" => twips(value).map(LineHeight::AtLeast),
        "exact" => twips(value).map(LineHeight::Exact),
        _ => None,
    }?;
    Some((twips(before)?, twips(after)?, line))
}

fn form_spacing_value(paragraph: &FlowParagraph) -> String {
    let line = match paragraph.line {
        LineHeight::Auto(value) => format!("auto={value}"),
        LineHeight::AtLeast(value) => format!("min={}", value / 635),
        LineHeight::Exact(value) => format!("exact={}", value / 635),
    };
    format!("{};{};{line}", paragraph.space_before / 635, paragraph.space_after / 635)
}

fn form_paragraph_comment(paragraph: &FlowParagraph) -> String {
    let align = match paragraph.align {
        FlowAlign::Left => "left", FlowAlign::Center => "center", FlowAlign::Right => "right",
        FlowAlign::Justify => "justify", FlowAlign::Distribute => "distribute",
    };
    format!("{FORM_PARAGRAPH}{align};{};{} -->", form_spacing_value(paragraph), paragraph.mark_format.size.unwrap_or(0))
}

fn form_field_id(run: &FlowRun) -> Option<&str> {
    if !matches!(run.kind, FlowRunKind::PlainText { .. }) {
        return None;
    }
    let id = run.control.as_ref()?.tag.strip_prefix(crate::form_template::PREFIX)?;
    crate::form_template::valid_field_id(id).then_some(id)
}

fn build_table_block(
    mut header_cells: Vec<String>,
    alignments: Vec<Option<FlowAlign>>,
    body_rows: Vec<Vec<String>>,
    ctx: &mut ParseContext<'_>,
    depth: usize,
) -> Block {
    let col_count = header_cells
        .len()
        .max(alignments.len())
        .max(body_rows.iter().map(Vec::len).max().unwrap_or(0))
        .max(1);

    let layout = header_cells.first_mut().and_then(|cell| form_table_layout(cell, col_count));
    let first_row_header = layout.as_ref().map(|layout| layout.first_row_header).unwrap_or_else(|| !header_cells.first_mut().is_some_and(|cell| {
        let Some(text) = cell.strip_prefix(PPTX_NO_TABLE_HEADER) else { return false };
        *cell = text.trim_start().to_owned();
        true
    }));

    let total_width: Emu = 6 * EMU_PER_INCH;
    let col_w = (total_width / col_count as Emu) / 635 * 635;
    let columns = layout.as_ref().map_or_else(|| vec![col_w; col_count], |layout| layout.columns.clone());

    let line = FlowBorder {
        width: 12_700 / 2,
        color: if layout.is_some() { Rgb::BLACK } else { Rgb(0xBFBFBF) },
        style: BorderStyle::Single,
        space: 0,
        art: None,
    };

    let mut previous_merges = HashMap::new();
    let mut build_row = |mut cells_text: Vec<String>, is_header: bool, ctx: &mut ParseContext<'_>| -> FlowRow {
        let height = cells_text.first_mut().map_or(RowHeight::Auto, form_row_height);
        let styled_header = is_header && layout.is_none();
        let mut cells = Vec::with_capacity(col_count);
        let mut active_merges = HashMap::new();
        let mut c_idx = 0;
        while c_idx < col_count {
            let raw_cell = cells_text.get(c_idx).map(String::as_str).unwrap_or("");
            let (raw_cell, requested_merge) = if let Some(cell) = raw_cell.strip_suffix("<!-- vmerge:start -->") {
                (cell.trim_end(), VerticalMerge::Start)
            } else if let Some(cell) = raw_cell.strip_suffix("<!-- vmerge:continue -->") {
                (cell.trim_end(), VerticalMerge::Continue)
            } else {
                (raw_cell, VerticalMerge::None)
            };
            let (raw_cell, span) = table_colspan(raw_cell, &cells_text, c_idx);
            let (raw_cell, anchor) = raw_cell.strip_prefix("<!-- docx-anchor:")
                .and_then(|rest| rest.split_once(" -->"))
                .and_then(|(anchor, body)| Some((body.trim_start(), match anchor {
                    "top" => ibis_doc_model::CellAnchor::Top,
                    "center" => ibis_doc_model::CellAnchor::Center,
                    "bottom" => ibis_doc_model::CellAnchor::Bottom,
                    _ => return None,
                })))
                .unwrap_or_else(|| (raw_cell, layout.as_ref().map_or(ibis_doc_model::CellAnchor::Top, |layout| layout.anchor)));
            let merge = match requested_merge {
                VerticalMerge::Continue
                    if !form_paragraph_format(raw_cell, None).0.trim().is_empty() || previous_merges.get(&c_idx) != Some(&span) => VerticalMerge::None,
                merge => merge,
            };
            if merge != VerticalMerge::None {
                active_merges.insert(c_idx, span);
            }
            let align = alignments.get(c_idx).copied().flatten();
            let base_fmt = if styled_header {
                RunFormat {
                    bold: Some(true),
                    ..RunFormat::default()
                }
            } else {
                RunFormat::default()
            };
            let paragraph = |text: &str, ctx: &mut ParseContext<'_>| {
                let (text, direct, mark_format) = form_paragraph_format(text, align);
                let parts: Vec<&str> = text.split("<br>").collect();
                let mut runs = Vec::new();
                for (pi, part) in parts.iter().enumerate() {
                    if pi > 0 {
                        runs.push(FlowRun::new(
                            FlowRunKind::LineBreak,
                            FlowRunStyle::default(),
                        ));
                    }
                    runs.extend(parse_inlines(part.trim(), ctx, &base_fmt));
                }
                let mut p = FlowParagraph {
                    id: ctx.fresh_id(),
                    style: ctx.styles.default_paragraph.clone(),
                    table_style: Some("TableGrid".to_string()),
                    direct,
                    mark_format,
                    runs,
                    ..FlowParagraph::default()
                };
                p.resolve(&ctx.styles);
                p.normalize();
                Block::Paragraph(p)
            };
            let append_paragraphs = |text: &str, ctx: &mut ParseContext<'_>, blocks: &mut Vec<Block>| {
                let separator = format!("<br>{FORM_PARAGRAPH}");
                let mut tail = text;
                while let Some(position) = tail.find(&separator) {
                    blocks.push(paragraph(&tail[..position], ctx));
                    tail = &tail[position + "<br>".len()..];
                }
                blocks.push(paragraph(tail, ctx));
            };
            let mut blocks = Vec::new();
            let mut remaining = raw_cell;
            while depth < 8 {
                let Some((before, payload)) = remaining.split_once(NESTED_TABLE_START) else { break };
                let Some((payload, after)) = payload.split_once(NESTED_TABLE_END) else { break };
                let Some(table) = parse_nested_table(payload, ctx, depth + 1) else { break };
                let before = before.strip_suffix("<br>").unwrap_or(before);
                if !before.is_empty() {
                    append_paragraphs(before, ctx, &mut blocks);
                }
                blocks.push(table);
                remaining = after.strip_prefix("<br>").unwrap_or(after);
            }
            // Word requires a paragraph at the end of a table cell.
            append_paragraphs(remaining, ctx, &mut blocks);
            cells.push(FlowCell {
                id: ctx.fresh_id(),
                blocks,
                span: span as u32,
                merge,
                shading: styled_header.then_some(Rgb(0xF2F4F8)),
                borders: [Some(line), Some(line), Some(line), Some(line)],
                own_borders: [None; 4],
                diagonals: [None; 2],
                anchor,
                direction: ibis_doc_model::CellDirection::LrTb,
                margins: [0, 108 * 635, 0, 108 * 635],
                no_wrap: false,
                ..FlowCell::default()
            });
            c_idx += span;
        }
        previous_merges = active_merges;
        FlowRow {
            id: ctx.fresh_id(),
            height,
            header: is_header,
            cant_split: false,
            cells,
            ..FlowRow::default()
        }
    };

    let mut rows = Vec::with_capacity(1 + body_rows.len());
    rows.push(build_row(header_cells, first_row_header, ctx));
    for r in body_rows {
        rows.push(build_row(r, false, ctx));
    }

    let mut table = FlowTable {
        id: ctx.fresh_id(),
        columns,
        rows,
        indent: 0,
        align: layout.as_ref().map_or(FlowAlign::Left, |layout| layout.align),
        cell_spacing: None,
        borders: [Some(Some(line)); 6],
        style: Some(TableStyleRef {
            id: "TableGrid".to_string(),
            look: TableLook {
                first_row: first_row_header,
                last_row: false,
                first_column: false,
                last_column: false,
                no_hband: false,
                no_vband: true,
            },
            row_band: 1,
            col_band: 1,
        }),
        autofit: None,
        floating: None,
        ..FlowTable::default()
    };
    table.apply_style(&ctx.styles);
    Block::Table(table)
}

// ============================================================================
// Native Office Chart block (` ```chart `)
// ============================================================================

fn build_chart_block(content: &str, ctx: &mut ParseContext<'_>) -> Result<Block, Error> {
    let mut kind = NewChartKind::ClusteredColumn;
    let mut title: Option<String> = None;
    let mut size = DEFAULT_CHART_SIZE;
    let mut categories: Vec<String> = Vec::new();
    let mut series_list: Vec<(String, Vec<Option<Number>>)> = Vec::new();
    let mut csv_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ") {
            // YAML series item: `- name: 2025, values: [10, 20, 30]` or `- 2025: [10, 20, 30]`
            if let Some((sname, svals)) = parse_yaml_series_item(rest) {
                series_list.push((sname, svals));
                continue;
            }
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            match key.as_str() {
                "type" | "kind" | "chart" => {
                    kind = parse_chart_kind(val);
                    continue;
                }
                "title" => {
                    let t = unquote_yaml(val);
                    if !t.is_empty() {
                        title = Some(t);
                    }
                    continue;
                }
                "width" => {
                    if let Some(w) = parse_length_emu(val) {
                        size.width = w.max(EMU_PER_INCH);
                    }
                    continue;
                }
                "height" => {
                    if let Some(h) = parse_length_emu(val) {
                        size.height = h.max(EMU_PER_INCH);
                    }
                    continue;
                }
                "categories" | "labels" | "x" => {
                    categories = parse_yaml_string_list(val);
                    continue;
                }
                "series" | "data" => {
                    continue;
                }
                _ => {}
            }
        }
        if trimmed.contains(',') {
            csv_lines.push(trimmed);
        }
    }

    // If CSV lines were provided instead of YAML `series:`, parse them.
    if series_list.is_empty() && !csv_lines.is_empty() {
        let rows: Vec<Vec<String>> = csv_lines
            .iter()
            .map(|l| l.split(',').map(|c| unquote_yaml(c.trim())).collect())
            .collect();
        if rows.len() >= 2 && rows[0].len() >= 2 {
            let header = &rows[0];
            categories = rows[1..]
                .iter()
                .map(|r| r.first().cloned().unwrap_or_default())
                .collect();
            for col in 1..header.len() {
                let name = header[col].clone();
                let values = rows[1..]
                    .iter()
                    .map(|r| {
                        r.get(col)
                            .and_then(|s| s.trim().parse::<f64>().ok())
                            .map(Number)
                    })
                    .collect();
                series_list.push((name, values));
            }
        }
    }

    build_chart_block_from_spec(kind, title, size, categories, series_list, &[], false, None, None, content, ctx)
}

/// `pie showData` labels the slices. A mix of `bar` and `line` becomes one
/// group per series, in source order, so a later `bar` stays a column.
fn apply_mermaid_series_layout(chart: &mut Chart, series_kinds: &[NewChartKind], show_data: bool) {
    if show_data {
        for group in &mut chart.groups {
            if matches!(group.kind, ChartType::Pie { .. } | ChartType::Doughnut { .. }) {
                for series in &mut group.series {
                    series.labels = Some(DataLabels {
                        show_value: true,
                        show_percent: true,
                        position: Some(LabelPosition::BestFit),
                        ..DataLabels::default()
                    });
                }
            }
        }
    }
    let has_line = series_kinds.iter().any(|kind| *kind == NewChartKind::Line);
    let has_column = series_kinds.iter().any(|kind| *kind != NewChartKind::Line);
    if !has_line || !has_column {
        return;
    }
    let flat: Vec<Series> = chart.groups.iter().flat_map(|group| group.series.clone()).collect();
    if flat.len() != series_kinds.len() {
        return;
    }
    let bar_kind = chart
        .groups
        .iter()
        .find_map(|group| match group.kind {
            ChartType::Bar { horizontal: false, gap_width, overlap } => {
                Some(ChartType::Bar { horizontal: false, gap_width, overlap })
            }
            _ => None,
        })
        .unwrap_or(ChartType::Bar { horizontal: false, gap_width: 219, overlap: -27 });
    let axes = chart.groups.first().map(|group| group.axes.clone()).unwrap_or_default();
    let grouping = chart.groups.first().map(|group| group.grouping).unwrap_or_default();
    chart.groups = flat
        .into_iter()
        .enumerate()
        .map(|(index, series)| ChartGroup {
            kind: if series_kinds.get(index) == Some(&NewChartKind::Line) { ChartType::Line } else { bar_kind },
            of_pie: None,
            grouping,
            series: vec![series],
            axes: axes.clone(),
            three_d: None,
        })
        .collect();
}

/// The frontmatter `config` of a Mermaid `pie` / `xychart` on the native chart: series or slice
/// colours, value labels, the doughnut hole and the highlighted slice.
fn apply_mermaid_chart_style(chart: &mut Chart, style: &crate::mermaid::MermaidChartStyle, categories: &[String]) {
    if let Some(crate::mermaid::PieLegendPosition::Side(side)) = style.pie_legend
        && let Some(legend) = &mut chart.legend
    {
        legend.side = side;
        legend.source_position = Some(match side {
            Side::Top => LegendPosition::Top,
            Side::Bottom => LegendPosition::Bottom,
            Side::Left => LegendPosition::Left,
            Side::Right => LegendPosition::Right,
        });
    }
    let solid = |hex: &str| parse_hex_rgb(hex).map(|rgb| Fill::Solid(ibis_doc_model::Color::opaque(rgb)));
    let mut series_index = 0usize;
    for group in &mut chart.groups {
        let is_pie = matches!(group.kind, ChartType::Pie { .. } | ChartType::Doughnut { .. });
        if let (ChartType::Doughnut { hole, .. }, Some(ratio)) = (&mut group.kind, style.donut_hole) {
            *hole = ((ratio * 100.0).round() as u32).clamp(10, 90);
        }
        let is_line = matches!(group.kind, ChartType::Line);
        for series in &mut group.series {
            if is_pie {
                if !style.palette.is_empty() {
                    series.point_fills = (0..categories.len())
                        .filter_map(|i| solid(&style.palette[i % style.palette.len()]).map(|fill| (i, fill)))
                        .collect();
                }
                if let Some(at) = style.highlight.as_ref().and_then(|h| categories.iter().position(|c| c == h)) {
                    series.point_explosions = vec![(at, 15)];
                }
            } else {
                let color = style
                    .palette
                    .get(series_index % style.palette.len().max(1))
                    .and_then(|hex| parse_hex_rgb(hex))
                    .map(ibis_doc_model::Color::opaque);
                if let Some(color) = color {
                    if is_line {
                        match &mut series.stroke {
                            Some(stroke) => stroke.color = color,
                            None => series.stroke = Some(ibis_doc_model::Stroke::solid(color, 28_575)),
                        }
                    }
                    series.fill = Fill::Solid(color);
                }
                if style.data_labels {
                    series.labels = Some(DataLabels {
                        show_value: true,
                        position: Some(if is_line {
                            LabelPosition::Above
                        } else if style.data_labels_outside {
                            LabelPosition::OutsideEnd
                        } else {
                            LabelPosition::InsideEnd
                        }),
                        ..DataLabels::default()
                    });
                }
                series_index += 1;
            }
        }
    }
}

/// `xychart` `y-axis min --> max` fixes the value axis' scale.
fn apply_mermaid_value_range(chart: &mut Chart, y_range: Option<(f64, f64)>) {
    let Some((min, max)) = y_range else { return };
    for axis in chart.axes.iter_mut().filter(|axis| axis.kind == AxisKind::Value) {
        axis.min = Some(Number(min));
        axis.max = Some(Number(max));
    }
}

/// The value axis' fixed range, when the chart has one.
fn chart_value_range(chart: &Chart) -> Option<(f64, f64)> {
    let axis = chart.axes.iter().find(|axis| axis.kind == AxisKind::Value)?;
    Some((axis.min?.0, axis.max?.0))
}

/// A slide copy of a combo keeps each source group's kind and series count.
fn copy_chart_group_topology(chart: &mut Chart, src: &Chart) {
    if src.groups.len() <= 1 {
        return;
    }
    let flat: Vec<Series> = chart.groups.iter().flat_map(|group| group.series.clone()).collect();
    let src_count: usize = src.groups.iter().map(|group| group.series.len()).sum();
    if flat.len() != src_count {
        return;
    }
    let axes = chart.groups.first().map(|group| group.axes.clone()).unwrap_or_default();
    let mut offset = 0;
    let mut groups = Vec::new();
    for src_group in &src.groups {
        let count = src_group.series.len();
        groups.push(ChartGroup {
            kind: src_group.kind,
            of_pie: src_group.of_pie.clone(),
            grouping: src_group.grouping,
            series: flat[offset..offset + count].to_vec(),
            axes: if axes.is_empty() { src_group.axes.clone() } else { axes.clone() },
            three_d: src_group.three_d,
        });
        offset += count;
    }
    chart.groups = groups;
}

fn build_mermaid_block(content: &str, ctx: &mut ParseContext<'_>) -> Result<Block, Error> {
    match crate::mermaid::render_mermaid(content) {
        Some(crate::mermaid::MermaidOutput::Chart(spec)) => {
            if ctx.source.is_some() && ctx.baseline_pres.is_some() {
                // `xyChart.width` / `height` keep their aspect ratio at the default chart width.
                let size = match spec.style.size_px {
                    Some((w, h)) if w > 0 && h > 0 => EmuSize::new(
                        DEFAULT_CHART_SIZE.width,
                        DEFAULT_CHART_SIZE.width * h as Emu / w as Emu,
                    ),
                    _ => DEFAULT_CHART_SIZE,
                };
                build_chart_block_from_spec(
                    spec.kind,
                    spec.title,
                    size,
                    spec.categories,
                    spec.series,
                    &spec.series_kinds,
                    spec.show_data,
                    spec.y_range,
                    Some(&spec.style),
                    content,
                    ctx,
                )
            } else if let Some((bytes, width_px, height_px)) =
                crate::mermaid::render_chart_spec_svg(&spec)
            {
                Ok(build_mermaid_svg_paragraph(bytes, width_px, height_px, content, ctx))
            } else {
                Ok(build_mermaid_code_block(content, ctx))
            }
        }
        Some(crate::mermaid::MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        }) => Ok(build_mermaid_svg_paragraph(
            bytes,
            width_px,
            height_px,
            content,
            ctx,
        )),
        None => Ok(build_mermaid_code_block(content, ctx)),
    }
}

// Keep the delimiters in editable text so both Office formats can store the
// language and the source together, including edits made to the code.
fn build_mermaid_code_block(content: &str, ctx: &mut ParseContext<'_>) -> Block {
    let mut fenced = String::new();
    write_mermaid_fence(content, &mut fenced);
    build_code_block(fenced.trim_end_matches('\n'), ctx)
}

const BIBLIOGRAPHY_FENCE: &str = "word-bibliography";

fn bibliography_fence(text: &str) -> Option<(&str, &str)> {
    let (first, body) = text.trim_start_matches(['\n', '\r']).split_once('\n')?;
    let (marker, info) = opening_fence(first.trim_end())?;
    if info.trim() != BIBLIOGRAPHY_FENCE {
        return None;
    }
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if closes_fence(line.trim(), marker) {
            let xml = body[..offset].trim();
            return crate::bibliography::is_sources(xml).then_some((xml, &body[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

fn opening_fence(line: &str) -> Option<(&str, &str)> {
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let count = line.bytes().take_while(|&byte| byte == marker).count();
    (count >= 3).then(|| line.split_at(count))
}

fn closes_fence(line: &str, marker: &str) -> bool {
    line.strip_prefix(marker).is_some_and(|tail| {
        tail.trim_end().bytes().all(|byte| byte == marker.as_bytes()[0])
    })
}

/// Track fenced content when scanning slide separators or speaker notes.
fn is_fenced_line<'a>(line: &'a str, fence: &mut Option<&'a str>) -> bool {
    if let Some(marker) = *fence {
        if closes_fence(line, marker) {
            *fence = None;
        }
        true
    } else if let Some((marker, _)) = opening_fence(line) {
        *fence = Some(marker);
        true
    } else {
        false
    }
}

fn is_literal_mermaid_fence(text: &str) -> bool {
    let Some((first, remaining)) = text.split_once('\n') else { return false };
    let Some((marker, info)) = opening_fence(first) else { return false };
    if info != "mermaid" { return false; }
    let mut lines = remaining.lines();
    let Some(last) = lines.next_back() else { return false };
    closes_fence(last, marker) && !lines.any(|line| closes_fence(line.trim(), marker))
}

fn build_mermaid_svg_paragraph(
    bytes: Vec<u8>,
    width_px: u32,
    height_px: u32,
    content: &str,
    ctx: &mut ParseContext<'_>,
) -> Block {
    const EMU_PER_PX: i64 = 9_525;
    let raw_w = (width_px.max(1) as i64) * EMU_PER_PX;
    let raw_h = (height_px.max(1) as i64) * EMU_PER_PX;
    let max_w = 6 * EMU_PER_INCH;
    let (width, height) = if raw_w > max_w {
        let scaled_h = (raw_h * max_w / raw_w).max(EMU_PER_PT * 10);
        (max_w, scaled_h)
    } else {
        (raw_w, raw_h)
    };
    let media_id = ctx.fresh_media_id();
    ctx.media.push(Media {
        id: media_id,
        format: MediaFormat::Svg,
        data: bytes.into(),
    });
    let mut look = PictureLook::default();
    look.alt_text.title = "__mermaid__".to_string();
    look.alt_text.description = content.to_string();
    let mut p = FlowParagraph {
        id: ctx.fresh_id(),
        style: ctx.styles.default_paragraph.clone(),
        direct: ParaFormat {
            align: Some(FlowAlign::Center),
            before: Some(6 * EMU_PER_PT),
            after: Some(6 * EMU_PER_PT),
            ..ParaFormat::default()
        },
        runs: vec![FlowRun::new(
            FlowRunKind::Picture {
                media: media_id,
                size: EmuSize::new(width, height),
                look,
            },
            FlowRunStyle::default(),
        )],
        ..FlowParagraph::default()
    };
    p.resolve(&ctx.styles);
    Block::Paragraph(p)
}

fn build_chart_block_from_spec(
    kind: NewChartKind,
    title: Option<String>,
    size: EmuSize,
    categories: Vec<String>,
    series_list: Vec<(String, Vec<Option<Number>>)>,
    series_kinds: &[NewChartKind],
    show_data: bool,
    y_range: Option<(f64, f64)>,
    style: Option<&crate::mermaid::MermaidChartStyle>,
    raw_fallback: &str,
    ctx: &mut ParseContext<'_>,
) -> Result<Block, Error> {
    let shape_id = ctx.fresh_shape_id();
    let (Some(source), Some(baseline_pres)) = (ctx.source, ctx.baseline_pres) else {
        // Without a DOCX source (e.g. clipboard fragment paste), render as a fenced code fallback.
        return Ok(build_code_block(raw_fallback, ctx));
    };
    let base_para_id = baseline_pres
        .flow
        .as_ref()
        .and_then(|f| {
            f.body.iter().find_map(|b| match b {
                Block::Paragraph(p) => Some(p.id),
                _ => None,
            })
        })
        .unwrap_or(1);

    let mut shape = source.new_doc_chart(
        baseline_pres,
        FlowPos::new(base_para_id, 0),
        shape_id,
        size,
        kind,
    )?;

    if let ShapeKind::Chart(chart_box) = &mut shape.kind {
        let mut chart = (**chart_box).clone();
        let has_custom_data = !series_list.is_empty();
        if has_custom_data {
            let temp_series: Vec<Series> = series_list
                .iter()
                .map(|(name, values)| Series {
                    name: name.clone(),
                    categories: categories.clone(),
                    values: values.clone(),
                    x_values: Vec::new(),
                    bubble_sizes: Vec::new(),
                    format: "General".to_string(),
                    fill: Fill::None,
                    stroke: None,
                    marker: None,
                    explosion: 0,
                    inner_shadow: None,
                    point_fills: Vec::new(),
                    point_strokes: Vec::new(),
                    point_explosions: Vec::new(),
                    labels: None,
                    trendline: None,
                    error_bars: None,
                    bubble_3d: false,
                })
                .collect();
            let temp_chart = Chart {
                groups: vec![ChartGroup {
                    kind: ChartType::Bar {
                        horizontal: false,
                        gap_width: 150,
                        overlap: 0,
                    },
                    of_pie: None,
                    grouping: Grouping::Standard,
                    series: temp_series,
                    axes: Vec::new(),
                    three_d: None,
                }],
                ..Chart::empty()
            };
            let chart_data = if style.is_some() {
                crate::save::pptx_chart_data::workbook_from_mermaid_chart(&temp_chart)?
            } else {
                crate::save::pptx_chart_data::workbook_from_chart(&temp_chart)?
            };
            let wb_pres = crate::read(&chart_data.workbook)?;
            chart = crate::save::pptx_chart_data::chart_from_data(
                &chart,
                &wb_pres,
                &chart_data.series,
            );
            if let Some(t) = &title {
                chart.title = Some(ChartTitle {
                    text: t.clone(),
                    style: ChartText::default(),
                    at: None,
                });
            }
            apply_mermaid_series_layout(&mut chart, series_kinds, show_data);
            apply_mermaid_value_range(&mut chart, y_range);
            if let Some(style) = style {
                apply_mermaid_chart_style(&mut chart, style, &categories);
            }
            chart.mermaid_source = ibis_doc_model::MermaidFence::new(raw_fallback);
            source.set_chart_data(shape_id, chart.clone(), chart_data);
        } else if let Some(t) = &title {
            chart.title = Some(ChartTitle {
                text: t.clone(),
                style: ChartText::default(),
                at: None,
            });
        }
        chart.mermaid_source = ibis_doc_model::MermaidFence::new(raw_fallback);
        *chart_box = Box::new(chart);
    }

    let mut p = FlowParagraph {
        id: ctx.fresh_id(),
        style: ctx.styles.default_paragraph.clone(),
        direct: ParaFormat {
            align: Some(FlowAlign::Center),
            before: Some(6 * EMU_PER_PT),
            after: Some(6 * EMU_PER_PT),
            ..ParaFormat::default()
        },
        runs: vec![FlowRun::new(
            FlowRunKind::Shape(Box::new(shape)),
            FlowRunStyle::default(),
        )],
        ..FlowParagraph::default()
    };
    p.resolve(&ctx.styles);
    Ok(Block::Paragraph(p))
}

fn parse_chart_kind(s: &str) -> NewChartKind {
    match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "column" | "clustered-column" | "縦棒" => NewChartKind::ClusteredColumn,
        "stacked-column" | "積み上げ縦棒" => NewChartKind::StackedColumn,
        "percent-stacked-column" | "100%-stacked-column" => NewChartKind::PercentStackedColumn,
        "bar" | "clustered-bar" | "横棒" => NewChartKind::ClusteredBar,
        "stacked-bar" | "積み上げ横棒" => NewChartKind::StackedBar,
        "percent-stacked-bar" | "100%-stacked-bar" => NewChartKind::PercentStackedBar,
        "line" | "折れ線" => NewChartKind::Line,
        "stacked-line" => NewChartKind::StackedLine,
        "area" | "面" => NewChartKind::Area,
        "stacked-area" => NewChartKind::StackedArea,
        "pie" | "円" => NewChartKind::Pie,
        "doughnut" | "donut" | "ドーナツ" => NewChartKind::Doughnut,
        "radar" | "レーダー" => NewChartKind::Radar,
        "scatter" | "散布図" => NewChartKind::Scatter,
        _ => NewChartKind::ClusteredColumn,
    }
}

fn parse_yaml_string_list(s: &str) -> Vec<String> {
    let inner = s.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|item| unquote_yaml(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}

fn parse_yaml_number_list(s: &str) -> Vec<Option<Number>> {
    let inner = s.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|item| item.trim().parse::<f64>().ok().map(Number))
        .collect()
}

fn parse_yaml_series_item(s: &str) -> Option<(String, Vec<Option<Number>>)> {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}');
    if let Some(vals_idx) = s.find("values:") {
        let before = s[..vals_idx].trim().trim_end_matches(',');
        let name = before
            .strip_prefix("name:")
            .map(|n| unquote_yaml(n.trim()))
            .unwrap_or_else(|| unquote_yaml(before));
        let vals_str = s[vals_idx + "values:".len()..].trim();
        return Some((name, parse_yaml_number_list(vals_str)));
    }
    if let Some((k, v)) = s.split_once(':') {
        let name = unquote_yaml(k.trim());
        let vals = parse_yaml_number_list(v.trim());
        if !vals.is_empty() {
            return Some((name, vals));
        }
    }
    None
}

// ============================================================================
// Inline parsing
// ============================================================================

fn markdown_sequence(name: &str) -> Option<(&'static str, &'static str)> {
    [("equation", "Equation"), ("figure", "Figure"), ("table", "Table")]
        .into_iter()
        .find(|(markdown, _)| name.eq_ignore_ascii_case(markdown))
}

fn markdown_bibliography_field(line: &str) -> Option<String> {
    if line == "[[bibliography]]" {
        return Some("BIBLIOGRAPHY".into());
    }
    let locale = line.strip_prefix("[[bibliography:")?.strip_suffix("]]")?;
    locale.parse::<u16>().ok()?;
    Some(format!(r"BIBLIOGRAPHY \l {locale}"))
}

fn format_markdown_bibliography(instruction: &str) -> Option<String> {
    let words = ibis_doc_model::flow_command::field_words(instruction);
    let [command, switches @ ..] = words.as_slice() else { return None };
    if !command.eq_ignore_ascii_case("BIBLIOGRAPHY") {
        return None;
    }
    match switches {
        [] => Some("[[bibliography]]".into()),
        [r"\l", locale] if locale.parse::<u16>().is_ok() => Some(format!("[[bibliography:{locale}]]")),
        _ => None,
    }
}

fn generated_bibliography_end(body: &[Block], start: usize) -> Option<usize> {
    for (index, block) in body.iter().enumerate().skip(start) {
        let Block::Paragraph(paragraph) = block else { return None };
        if index != start && paragraph.field_start.is_some() {
            return None;
        }
        if paragraph.field_end {
            return Some(index);
        }
    }
    None
}

fn citation_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.bytes().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-' | b'.'))
}

fn citation_pages(pages: &str) -> bool {
    let number = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    match pages.split_once(['-', '–']) {
        Some((first, last)) => number(first) && number(last),
        None => number(pages),
    }
}

fn markdown_citation_instruction(value: &str) -> Option<String> {
    let mut sources = value.split(';').map(str::trim);
    let mut instruction = String::from("CITATION ");
    instruction.push_str(&markdown_citation_source(sources.next()?)?);
    for source in sources {
        instruction.push_str(r" \m ");
        instruction.push_str(&markdown_citation_source(source)?);
    }
    Some(instruction)
}

fn markdown_citation_source(value: &str) -> Option<String> {
    let (source, pages) = value.split_once('#').map_or((value, None), |(source, pages)| (source, Some(pages)));
    let (tag, locale) = source.split_once(':').map_or((source, None), |(tag, locale)| (tag, Some(locale)));
    if !citation_tag(tag) || locale.is_some_and(|value| value.parse::<u16>().is_err())
        || pages.is_some_and(|value| !citation_pages(value))
    {
        return None;
    }
    let mut instruction = tag.to_owned();
    if let Some(locale) = locale {
        instruction.push_str(&format!(r" \l {locale}"));
    }
    if let Some(pages) = pages {
        instruction.push_str(&format!(r#" \p "{pages}""#));
    }
    Some(instruction)
}

fn format_markdown_citation<'a>(mut tag: &'a str, switches: &[&'a str]) -> Option<String> {
    let mut marker = String::from("{{cite:");
    let mut start = 0;
    for (index, pair) in switches.chunks(2).enumerate() {
        if let [r"\m", next_tag] = pair {
            let end = index * 2;
            marker.push_str(&format_markdown_citation_source(tag, &switches[start..end])?);
            marker.push(';');
            tag = next_tag;
            start = end + 2;
        }
    }
    marker.push_str(&format_markdown_citation_source(tag, &switches[start..])?);
    marker.push_str("}}");
    Some(marker)
}

fn format_markdown_citation_source(tag: &str, switches: &[&str]) -> Option<String> {
    fn unquote(value: &str) -> &str {
        value.strip_prefix('"').and_then(|value| value.strip_suffix('"')).unwrap_or(value)
    }
    let tag = unquote(tag);
    if !citation_tag(tag) {
        return None;
    }
    let mut locale = None;
    let mut pages = None;
    for pair in switches.chunks(2) {
        match pair {
            [r"\l", value] if locale.is_none() && value.parse::<u16>().is_ok() => locale = Some(*value),
            [r"\p", value] if pages.is_none() && citation_pages(unquote(value)) => pages = Some(unquote(value)),
            _ => return None,
        }
    }
    let mut marker = tag.to_owned();
    if let Some(locale) = locale {
        marker.push(':');
        marker.push_str(locale);
    }
    if let Some(pages) = pages {
        marker.push('#');
        marker.push_str(pages);
    }
    Some(marker)
}

const PAGE_FIELDS: [(&str, &str); 4] = [
    ("page", "PAGE"),
    ("pages", "NUMPAGES"),
    ("section", "SECTION"),
    ("section-pages", "SECTIONPAGES"),
];

fn markdown_page_field(value: &str) -> Option<String> {
    let (name, format) = value.split_once(':').map_or((value, None), |(name, format)| (name, Some(format)));
    if name == "page-ref" {
        let bookmark = format.filter(|name| is_equation_bookmark(name))?;
        return Some(format!(r"PAGEREF {bookmark} \h"));
    }
    let (_, command) = PAGE_FIELDS.iter().find(|(label, _)| *label == name)?;
    match format {
        Some(format) if matches!(format, "Arabic" | "ROMAN" | "roman" | "ALPHABETIC" | "alphabetic") => {
            Some(format!(r"{command} \* {format}"))
        }
        None => Some((*command).to_owned()),
        _ => None,
    }
}

fn format_markdown_page_field(words: &[&str]) -> Option<String> {
    let command = words.first()?;
    let (label, _) = PAGE_FIELDS.iter().find(|(_, keyword)| keyword.eq_ignore_ascii_case(command))?;
    match words {
        [_] => Some(format!("{{{{field:{label}}}}}")),
        [_, r"\*", format] if matches!(*format, "Arabic" | "ROMAN" | "roman" | "ALPHABETIC" | "alphabetic") => {
            Some(format!("{{{{field:{label}:{format}}}}}"))
        }
        _ => None,
    }
}

fn format_markdown_page_ref(bookmark: &str, shown: &str, locked: bool) -> String {
    if !locked && is_equation_bookmark(bookmark) {
        format!("{{{{field:page-ref:{bookmark}}}}}")
    } else {
        shown.to_owned()
    }
}

fn markdown_inline_field(rest: &str) -> Option<(usize, String)> {
    let field = rest.strip_prefix("{{")?;
    let (token, _) = field.split_once("}}")?;
    let (kind, value) = token.split_once(':')?;
    let instruction = match kind {
        "seq" => format!("SEQ {}", markdown_sequence(value)?.1),
        "ref" if is_equation_bookmark(value) => format!("REF {value} \\h"),
        "cite" => markdown_citation_instruction(value)?,
        "field" => markdown_page_field(value)?,
        _ => return None,
    };
    Some((token.len() + 4, instruction))
}

fn format_markdown_inline_field(instruction: &str, shown: &str) -> String {
    let words = ibis_doc_model::flow_command::field_words(instruction);
    match words.as_slice() {
        [seq, name] if seq.eq_ignore_ascii_case("SEQ") => {
            markdown_sequence(name).map_or_else(|| shown.to_owned(), |(label, _)| format!("{{{{seq:{label}}}}}"))
        }
        [reference, name, r"\h"] if reference.eq_ignore_ascii_case("REF") && is_equation_bookmark(name) => {
            format!("{{{{ref:{name}}}}}")
        }
        [citation, tag, switches @ ..] if citation.eq_ignore_ascii_case("CITATION") => {
            format_markdown_citation(tag, switches).unwrap_or_else(|| shown.to_owned())
        }
        _ => format_markdown_page_field(&words).unwrap_or_else(|| shown.to_owned()),
    }
}

fn parse_inlines(text: &str, ctx: &mut ParseContext<'_>, base_fmt: &RunFormat) -> Vec<FlowRun> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    let bytes = text.as_bytes();

    let flush_plain = |plain: &mut String, out: &mut Vec<FlowRun>, fmt: &RunFormat| {
        if !plain.is_empty() {
            out.push(FlowRun {
                kind: FlowRunKind::Text(std::mem::take(plain)),
                style: FlowRunStyle::default(),
                format: fmt.clone(),
                char_style: None,
                link: None,
                revision: None,
                show_field_code: false,
                field_locked: false,
                control: None,
            });
        }
    };

    while i < text.len() {
        let rest = &text[i..];

        if let Some(length) = [XLSX_FORMULA_CELL_MARKER, XLSX_FORMAT_CELL_MARKER].into_iter()
            .find_map(|marker| {
                let (payload, _) = rest.strip_prefix(marker)?.split_once(XLSX_CELL_MARKER_END)?;
                Some(marker.len() + payload.len() + XLSX_CELL_MARKER_END.len())
            })
        {
            plain.push_str(&rest[..length]);
            i += length;
            continue;
        }

        // Escape sequence `\*`, `\[`, etc.
        if bytes[i] == b'\\' && i + 1 < text.len() {
            let next_ch = text[i + 1..].chars().next().unwrap();
            if next_ch.is_ascii_punctuation() {
                plain.push(next_ch);
                i += 1 + next_ch.len_utf8();
                continue;
            }
        }

        if let Some((size, body)) = rest.strip_prefix(FORM_RUN_SIZE).and_then(|s| s.split_once(" -->"))
            && let Ok(size) = size.parse::<u32>()
            && (1..=14400).contains(&size)
            && let Some((quoted, _)) = body.split_once(FORM_RUN_END)
            && !quoted.contains(FORM_RUN_SIZE)
        {
            flush_plain(&mut plain, &mut out, base_fmt);
            let format = RunFormat { size: Some(size), ..base_fmt.clone() };
            out.extend(parse_inlines(quoted, ctx, &format));
            i += rest.len() - body.len() + quoted.len() + FORM_RUN_END.len();
            continue;
        }

        if let Some((id, _)) = rest.strip_prefix("{{form:").and_then(|body| body.split_once("}}"))
            && crate::form_template::valid_field_id(id)
        {
            flush_plain(&mut plain, &mut out, base_fmt);
            let mut run = FlowRun::new(
                FlowRunKind::PlainText { text: "…".into(), placeholder: false },
                FlowRunStyle::default(),
            );
            run.format = base_fmt.clone();
            run.control = Some(Box::new(ibis_doc_model::ControlInfo {
                tag: format!("{}{id}", crate::form_template::PREFIX),
                ..Default::default()
            }));
            out.push(run);
            i += "{{form:".len() + id.len() + "}}".len();
            continue;
        }

        if let Some((consumed, instruction)) = markdown_inline_field(rest) {
            flush_plain(&mut plain, &mut out, base_fmt);
            let kind = if instruction == "PAGE" {
                FlowRunKind::PageNumber
            } else if let Some(bookmark) = instruction.strip_prefix("PAGEREF ").and_then(|rest| rest.strip_suffix(r" \h")) {
                FlowRunKind::PageRef { bookmark: bookmark.to_owned(), shown: "?".into() }
            } else {
                FlowRunKind::Field { instruction, shown: "?".into() }
            };
            let mut run = FlowRun::new(kind, FlowRunStyle::default());
            run.format = base_fmt.clone();
            out.push(run);
            i += consumed;
            continue;
        }

        // CriticMarkup Insertion: `{++text++}`
        if let Some(after) = rest.strip_prefix("{++") {
            if let Some(end) = after.find("++}") {
                flush_plain(&mut plain, &mut out, base_fmt);
                let inner = &after[..end];
                let (author, body) = split_critic_author(inner);
                let mut runs = parse_inlines(body, ctx, base_fmt);
                for r in &mut runs {
                    r.revision = Some(Revision {
                        kind: RevisionKind::Insert,
                        author: author.clone(),
                        date: "2026-01-01T00:00:00Z".to_string(),
                    });
                }
                out.extend(runs);
                i += 3 + end + 3;
                continue;
            }
        }

        // CriticMarkup Deletion: `{--text--}`
        if let Some(after) = rest.strip_prefix("{--") {
            if let Some(end) = after.find("--}") {
                flush_plain(&mut plain, &mut out, base_fmt);
                let inner = &after[..end];
                let (author, body) = split_critic_author(inner);
                let mut runs = parse_inlines(body, ctx, base_fmt);
                for r in &mut runs {
                    r.revision = Some(Revision {
                        kind: RevisionKind::Delete,
                        author: author.clone(),
                        date: "2026-01-01T00:00:00Z".to_string(),
                    });
                }
                out.extend(runs);
                i += 3 + end + 3;
                continue;
            }
        }

        // CriticMarkup Substitution: `{~~old~>new~~}`
        if let Some(after) = rest.strip_prefix("{~~") {
            if let Some(end) = after.find("~~}") {
                let inner = &after[..end];
                if let Some((old_part, new_part)) = inner.split_once("~>") {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let mut del_runs = parse_inlines(old_part, ctx, base_fmt);
                    for r in &mut del_runs {
                        r.revision = Some(Revision {
                            kind: RevisionKind::Delete,
                            author: "Author".to_string(),
                            date: "2026-01-01T00:00:00Z".to_string(),
                        });
                    }
                    let mut ins_runs = parse_inlines(new_part, ctx, base_fmt);
                    for r in &mut ins_runs {
                        r.revision = Some(Revision {
                            kind: RevisionKind::Insert,
                            author: "Author".to_string(),
                            date: "2026-01-01T00:00:00Z".to_string(),
                        });
                    }
                    out.extend(del_runs);
                    out.extend(ins_runs);
                    i += 3 + end + 3;
                    continue;
                }
            }
        }

        // CriticMarkup Comment: `{>>author: comment<<}`
        if let Some(after) = rest.strip_prefix("{>>") {
            if let Some(end) = after.find("<<}") {
                flush_plain(&mut plain, &mut out, base_fmt);
                let inner = after[..end].trim();
                let (author, comment_text) = split_critic_author(inner);
                let cid = ctx.fresh_comment_id();
                let mut cp = FlowParagraph {
                    id: ctx.fresh_id(),
                    style: ctx
                        .styles
                        .paragraph_style_named("annotation text")
                        .map(str::to_string)
                        .or_else(|| ctx.styles.default_paragraph.clone()),
                    runs: vec![FlowRun::new(
                        FlowRunKind::Text(comment_text.to_string()),
                        FlowRunStyle::default(),
                    )],
                    ..FlowParagraph::default()
                };
                cp.resolve(&ctx.styles);
                let initials: String = author.chars().take(2).collect();
                ctx.comments.push(Comment {
                    id: cid,
                    author,
                    initials,
                    date: "2026-01-01T00:00:00Z".to_string(),
                    blocks: vec![Block::Paragraph(cp)],
                    parent: None,
                    done: false,
                    para_id: Some(format!(
                        "{:08X}",
                        0x1C00_0000u32.wrapping_add(cid.wrapping_mul(0x0001_0101)) & 0x7FFF_FFFF
                    )),
                });
                // Wrap the preceding run in CommentStart..CommentEnd if available.
                let start_run = FlowRun {
                    kind: FlowRunKind::CommentStart(cid),
                    style: FlowRunStyle::default(),
                    format: RunFormat::default(),
                    char_style: None,
                    link: None,
                    revision: None,
                    show_field_code: false,
                    field_locked: false,
                    control: None,
                };
                let end_run = FlowRun {
                    kind: FlowRunKind::CommentEnd(cid),
                    style: FlowRunStyle::default(),
                    format: RunFormat::default(),
                    char_style: None,
                    link: None,
                    revision: None,
                    show_field_code: false,
                    field_locked: false,
                    control: None,
                };
                let ref_run = FlowRun::new(FlowRunKind::CommentRef(cid), FlowRunStyle::default());
                if let Some(last) = out.pop() {
                    out.push(start_run);
                    out.push(last);
                    out.push(end_run);
                    out.push(ref_run);
                } else {
                    out.push(start_run);
                    out.push(end_run);
                    out.push(ref_run);
                }
                i += 3 + end + 3;
                continue;
            }
        }

        // Japanese Ruby: `{漢字|かんじ}`
        if let Some(after) = rest.strip_prefix('{') {
            if let Some(close) = after.find('}') {
                let inner = &after[..close];
                if !inner.contains('\n')
                    && let Some((base, ruby)) = inner.split_once('|')
                    && !base.is_empty()
                    && !ruby.is_empty()
                {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    out.push(FlowRun {
                        kind: FlowRunKind::Ruby {
                            base: base.to_string(),
                            text: ruby.to_string(),
                            align: RubyAlign::DistributeSpace,
                            size: 1000,
                            raise: 0,
                        },
                        style: FlowRunStyle::default(),
                        format: base_fmt.clone(),
                        char_style: None,
                        link: None,
                        revision: None,
                        show_field_code: false,
                        field_locked: false,
                        control: None,
                    });
                    i += 1 + close + 1;
                    continue;
                }
            }
        }

        // Narou / Aozora Ruby: `｜漢字《かんじ》`
        if let Some(after) = rest.strip_prefix('｜') {
            if let Some(open_rt) = after.find('《') {
                if !after[open_rt + '《'.len_utf8()..].starts_with('《') {
                    if let Some(close_rt) = after[open_rt + '《'.len_utf8()..].find('》') {
                        let base = &after[..open_rt];
                        let ruby = &after[open_rt + '《'.len_utf8()..open_rt + '《'.len_utf8() + close_rt];
                        if !base.is_empty() && !ruby.is_empty() {
                            flush_plain(&mut plain, &mut out, base_fmt);
                            out.push(FlowRun {
                                kind: FlowRunKind::Ruby {
                                    base: base.to_string(),
                                    text: ruby.to_string(),
                                    align: RubyAlign::DistributeSpace,
                                    size: 1000,
                                    raise: 0,
                                },
                                style: FlowRunStyle::default(),
                                format: base_fmt.clone(),
                                char_style: None,
                                link: None,
                                revision: None,
                                show_field_code: false,
                                field_locked: false,
                                control: None,
                            });
                            i += '｜'.len_utf8() + open_rt + '《'.len_utf8() + close_rt + '》'.len_utf8();
                            continue;
                        }
                    }
                }
            }
        }

        // Emphasis dots: `《《強調》》`
        if let Some(after) = rest.strip_prefix("《《") {
            if let Some(end) = after.find("》》") {
                let inner = &after[..end];
                if !inner.is_empty() {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let mut fmt = base_fmt.clone();
                    fmt.emphasis = Some(EmphasisMark::Dot);
                    out.extend(parse_inlines(inner, ctx, &fmt));
                    i += "《《".len() + end + "》》".len();
                    continue;
                }
            }
        }

        // Inline image: `![alt](data:image/...;base64,...){width=... height=...}`
        if let Some(after_bang) = rest.strip_prefix("![") {
            if let Some(alt_end) = after_bang.find("](") {
                let alt = &after_bang[..alt_end];
                let after_open_paren = &after_bang[alt_end + 2..];
                if let Some(url_end) = after_open_paren.find(')') {
                    let raw_target = after_open_paren[..url_end].trim();
                    let url = strip_markdown_link_title(raw_target);
                    let mut total_consumed = 2 + alt_end + 2 + url_end + 1;
                    let mut explicit_w: Option<Emu> = None;
                    let mut explicit_h: Option<Emu> = None;
                    let after_paren = &after_open_paren[url_end + 1..];
                    if let Some(attr_open) = after_paren.strip_prefix('{') {
                        if let Some(attr_close) = attr_open.find('}') {
                            let attr_str = &attr_open[..attr_close];
                            for part in attr_str.split_whitespace() {
                                if let Some(w) = part.strip_prefix("width=").and_then(parse_length_emu) {
                                    explicit_w = Some(w.max(EMU_PER_PT * 10));
                                } else if let Some(h) = part.strip_prefix("height=").and_then(parse_length_emu) {
                                    explicit_h = Some(h.max(EMU_PER_PT * 10));
                                }
                            }
                            total_consumed += 1 + attr_close + 1;
                        }
                    }
                    if let Some((format, data)) = decode_data_uri_image(url).or_else(|| load_local_image(url, ctx.base_dir)) {
                        let img_size = resolve_image_size(&data, format, explicit_w, explicit_h);
                        flush_plain(&mut plain, &mut out, base_fmt);
                        let media_id = ctx.fresh_media_id();
                        ctx.media.push(Media {
                            id: media_id,
                            format,
                            data: data.into(),
                        });
                        let mut look = PictureLook::default();
                        if !alt.is_empty() {
                            look.alt_text.description = alt.to_string();
                        }
                        out.push(FlowRun {
                            kind: FlowRunKind::Picture {
                                media: media_id,
                                size: img_size,
                                look,
                            },
                            style: FlowRunStyle::default(),
                            format: base_fmt.clone(),
                            char_style: None,
                            link: None,
                            revision: None,
                            show_field_code: false,
                            field_locked: false,
                            control: None,
                        });
                        i += total_consumed;
                        continue;
                    }
                }
            }
        }

        // Footnote reference: `[^id]`
        if let Some(after) = rest.strip_prefix("[^") {
            if let Some(close) = after.find(']') {
                let label = &after[..close];
                if !label.is_empty() && !label.contains(' ') && !after[close + 1..].starts_with(':') {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let id = ctx.footnote_id_for(label);
                    let mut fmt = base_fmt.clone();
                    fmt.position = Some(VerticalPosition::Superscript);
                    out.push(FlowRun {
                        kind: FlowRunKind::NoteRef {
                            endnote: markdown_endnote_id(label).is_some(),
                            id,
                        },
                        style: FlowRunStyle::default(),
                        format: fmt,
                        char_style: None,
                        link: None,
                        revision: None,
                        show_field_code: false,
                        field_locked: false,
                        control: None,
                    });
                    i += 2 + close + 1;
                    continue;
                }
            }
        }

        // Span attributes or Link: `[text]{.tcy}` / `[text]{.em}` / `[text](url)`
        if let Some(after_open) = rest.strip_prefix('[') {
            if let Some(close_bracket) = find_matching_bracket(after_open) {
                let inner_text = &after_open[..close_bracket];
                let after_bracket = &after_open[close_bracket + 1..];

                // `[text]{.tcy}` or `[text]{.em}`
                if let Some(attr_after) = after_bracket.strip_prefix('{') {
                    if let Some(attr_close) = attr_after.find('}') {
                        let attr_body = attr_after[..attr_close].trim();
                        if matches!(attr_body, ".tcy" | "tcy") {
                            flush_plain(&mut plain, &mut out, base_fmt);
                            let mut fmt = base_fmt.clone();
                            fmt.tate_chu_yoko = Some(true);
                            out.extend(parse_inlines(inner_text, ctx, &fmt));
                            i += 1 + close_bracket + 1 + 1 + attr_close + 1;
                            continue;
                        } else if matches!(attr_body, ".em" | ".emphasis" | "em") {
                            flush_plain(&mut plain, &mut out, base_fmt);
                            let mut fmt = base_fmt.clone();
                            fmt.emphasis = Some(EmphasisMark::Dot);
                            out.extend(parse_inlines(inner_text, ctx, &fmt));
                            i += 1 + close_bracket + 1 + 1 + attr_close + 1;
                            continue;
                        }
                    }
                }

                // `[text](url)`
                if let Some(paren_after) = after_bracket.strip_prefix('(') {
                    if let Some(close_paren) = paren_after.find(')') {
                        let target = paren_after[..close_paren].trim();
                        flush_plain(&mut plain, &mut out, base_fmt);
                        let link = if let Some(anchor) = target.strip_prefix('#') {
                            Link::Anchor(anchor.to_string())
                        } else {
                            Link::Url(target.to_string())
                        };
                        let mut link_fmt = base_fmt.clone();
                        if link_fmt.color.is_none() {
                            link_fmt.color = Some(Rgb(0x0563C1));
                        }
                        if link_fmt.underline.is_none() {
                            link_fmt.underline = Some(true);
                        }
                        let mut runs = parse_inlines(inner_text, ctx, &link_fmt);
                        for r in &mut runs {
                            r.link = Some(link.clone());
                        }
                        out.extend(runs);
                        i += 1 + close_bracket + 1 + 1 + close_paren + 1;
                        continue;
                    }
                }
            }
        }

        // Inline code: `` `code` ``
        if let Some(after) = rest.strip_prefix('`') {
            if let Some(end) = after.find('`') {
                if end > 0 {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let mut fmt = base_fmt.clone();
                    fmt.font = Some("Consolas".to_string());
                    fmt.h_ansi_font = Some("Consolas".to_string());
                    fmt.east_asian_font = Some("ＭＳ ゴシック".to_string());
                    fmt.highlight = Some(Some(Rgb(0xF2F4F8)));
                    out.push(FlowRun {
                        kind: FlowRunKind::Text(after[..end].to_string()),
                        style: FlowRunStyle::default(),
                        format: fmt,
                        char_style: None,
                        link: None,
                        revision: None,
                        show_field_code: false,
                        field_locked: false,
                        control: None,
                    });
                    i += 1 + end + 1;
                    continue;
                }
            }
        }

        // Inline math: `$math$`
        if let Some(after) = rest.strip_prefix('$') {
            if !after.starts_with('$') {
                if let Some(end) = after.find('$') {
                    let latex = &after[..end];
                    if !latex.is_empty() && !latex.starts_with(' ') && !latex.ends_with(' ') {
                        flush_plain(&mut plain, &mut out, base_fmt);
                        let math = parse_latex(latex);
                        out.push(FlowRun {
                            kind: FlowRunKind::Math {
                                math,
                                display: false,
                            },
                            style: FlowRunStyle::default(),
                            format: base_fmt.clone(),
                            char_style: None,
                            link: None,
                            revision: None,
                            show_field_code: false,
                            field_locked: false,
                            control: None,
                        });
                        i += 1 + end + 1;
                        continue;
                    }
                }
            }
        }

        // Bold + Italic: `***text***`
        if let Some(after) = rest.strip_prefix("***") {
            if let Some(end) = after.find("***") {
                if end > 0 {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let mut fmt = base_fmt.clone();
                    fmt.bold = Some(true);
                    fmt.italic = Some(true);
                    out.extend(parse_inlines(&after[..end], ctx, &fmt));
                    i += 3 + end + 3;
                    continue;
                }
            }
        }

        // Bold: `**text**`
        if let Some(after) = rest.strip_prefix("**") {
            if let Some(end) = after.find("**") {
                if end > 0 {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let mut fmt = base_fmt.clone();
                    fmt.bold = Some(true);
                    out.extend(parse_inlines(&after[..end], ctx, &fmt));
                    i += 2 + end + 2;
                    continue;
                }
            }
        }

        // Strikethrough: `~~text~~`
        if let Some(after) = rest.strip_prefix("~~") {
            if let Some(end) = after.find("~~") {
                if end > 0 {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let mut fmt = base_fmt.clone();
                    fmt.strike = Some(true);
                    out.extend(parse_inlines(&after[..end], ctx, &fmt));
                    i += 2 + end + 2;
                    continue;
                }
            }
        }

        // Underline: `++text++`
        if let Some(after) = rest.strip_prefix("++") {
            if let Some(end) = after.find("++") {
                if end > 0 {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let mut fmt = base_fmt.clone();
                    fmt.underline = Some(true);
                    out.extend(parse_inlines(&after[..end], ctx, &fmt));
                    i += 2 + end + 2;
                    continue;
                }
            }
        }

        // Highlight: `==text==`
        if let Some(after) = rest.strip_prefix("==") {
            if let Some(end) = after.find("==") {
                if end > 0 {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let mut fmt = base_fmt.clone();
                    fmt.highlight = Some(Some(Rgb(0xFFFF00)));
                    out.extend(parse_inlines(&after[..end], ctx, &fmt));
                    i += 2 + end + 2;
                    continue;
                }
            }
        }

        // Italic: `*text*`
        if let Some(after) = rest.strip_prefix('*') {
            if !after.starts_with('*') && !after.starts_with(' ') {
                if let Some(end) = after.find('*') {
                    if end > 0 {
                        flush_plain(&mut plain, &mut out, base_fmt);
                        let mut fmt = base_fmt.clone();
                        fmt.italic = Some(true);
                        out.extend(parse_inlines(&after[..end], ctx, &fmt));
                        i += 1 + end + 1;
                        continue;
                    }
                }
            }
        }

        // Superscript: `^text^`
        if let Some(after) = rest.strip_prefix('^') {
            if let Some(end) = after.find('^') {
                if end > 0 && !after[..end].contains(' ') {
                    flush_plain(&mut plain, &mut out, base_fmt);
                    let mut fmt = base_fmt.clone();
                    fmt.position = Some(VerticalPosition::Superscript);
                    out.extend(parse_inlines(&after[..end], ctx, &fmt));
                    i += 1 + end + 1;
                    continue;
                }
            }
        }

        // Subscript: `~text~`
        if let Some(after) = rest.strip_prefix('~') {
            if !after.starts_with('~') {
                if let Some(end) = after.find('~') {
                    if end > 0 && !after[..end].contains(' ') {
                        flush_plain(&mut plain, &mut out, base_fmt);
                        let mut fmt = base_fmt.clone();
                        fmt.position = Some(VerticalPosition::Subscript);
                        out.extend(parse_inlines(&after[..end], ctx, &fmt));
                        i += 1 + end + 1;
                        continue;
                    }
                }
            }
        }

        let ch = rest.chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }

    flush_plain(&mut plain, &mut out, base_fmt);
    out
}

fn find_matching_bracket(s: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_critic_author(s: &str) -> (String, &str) {
    if let Some((author, rest)) = s.split_once(':') {
        let a = author.trim();
        if !a.is_empty() && a.len() <= 32 && !a.contains('\n') {
            return (a.to_string(), rest.trim());
        }
    }
    ("Author".to_string(), s)
}

fn strip_markdown_link_title(target: &str) -> &str {
    let trimmed = target.trim();
    let trimmed = trimmed.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(trimmed);
    if let Some((path, _)) = trimmed.split_once(" \"") {
        path.trim()
    } else if let Some((path, _)) = trimmed.split_once(" '") {
        path.trim()
    } else {
        trimmed
    }
}

fn decode_data_uri_image(uri: &str) -> Option<(MediaFormat, Vec<u8>)> {
    let after_data = uri.strip_prefix("data:image/")?;
    let (mime_sub, b64) = after_data.split_once(";base64,")?;
    let format = match mime_sub.to_ascii_lowercase().as_str() {
        "png" => MediaFormat::Png,
        "jpeg" | "jpg" => MediaFormat::Jpeg,
        "gif" => MediaFormat::Gif,
        "bmp" => MediaFormat::Bmp,
        "tiff" => MediaFormat::Tiff,
        "webp" => MediaFormat::Webp,
        "svg+xml" | "svg" => MediaFormat::Svg,
        "x-emf" | "emf" => MediaFormat::Emf,
        "x-wmf" | "wmf" => MediaFormat::Wmf,
        _ => return None,
    };
    let bytes = decode_base64(b64.trim())?;
    Some((format, bytes))
}

/// Load a relative local image path against `base_dir` (non-WASM only, confined within `base_dir`).
#[cfg(not(target_family = "wasm"))]
fn load_local_image(url: &str, base_dir: Option<&std::path::Path>) -> Option<(MediaFormat, Vec<u8>)> {
    let base_dir = base_dir?;
    let clean = url.split(['?', '#']).next().unwrap_or(url).trim();
    if clean.is_empty()
        || clean.starts_with("http://")
        || clean.starts_with("https://")
        || clean.starts_with("data:")
        || clean.starts_with("file:")
    {
        return None;
    }
    let rel = std::path::Path::new(clean);
    if rel.is_absolute() {
        return None;
    }
    let candidate = base_dir.join(rel);
    let canon_base = std::fs::canonicalize(base_dir).ok()?;
    let canon_file = std::fs::canonicalize(&candidate).ok()?;
    if !canon_file.starts_with(&canon_base) {
        return None;
    }
    let bytes = std::fs::read(&canon_file).ok()?;
    let format = detect_image_format(&bytes, &canon_file)?;
    Some((format, bytes))
}

#[cfg(target_family = "wasm")]
fn load_local_image(_url: &str, _base_dir: Option<&std::path::Path>) -> Option<(MediaFormat, Vec<u8>)> {
    None
}

fn detect_image_format(bytes: &[u8], path: &std::path::Path) -> Option<MediaFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(MediaFormat::Png);
    }
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        return Some(MediaFormat::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(MediaFormat::Gif);
    }
    if bytes.starts_with(b"BM") {
        return Some(MediaFormat::Bmp);
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => Some(MediaFormat::Png),
        Some("jpg" | "jpeg") => Some(MediaFormat::Jpeg),
        Some("gif") => Some(MediaFormat::Gif),
        Some("bmp") => Some(MediaFormat::Bmp),
        Some("svg") => Some(MediaFormat::Svg),
        Some("emf") => Some(MediaFormat::Emf),
        Some("wmf") => Some(MediaFormat::Wmf),
        _ => None,
    }
}

fn infer_image_dimensions_px(data: &[u8], format: MediaFormat) -> Option<(u32, u32)> {
    match format {
        MediaFormat::Png if data.len() >= 24 && data.starts_with(b"\x89PNG\r\n\x1a\n") => {
            let w = u32::from_be_bytes(data[16..20].try_into().ok()?);
            let h = u32::from_be_bytes(data[20..24].try_into().ok()?);
            (w > 0 && h > 0).then_some((w, h))
        }
        MediaFormat::Gif if data.len() >= 10 && (data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) => {
            let w = u16::from_le_bytes(data[6..8].try_into().ok()?) as u32;
            let h = u16::from_le_bytes(data[8..10].try_into().ok()?) as u32;
            (w > 0 && h > 0).then_some((w, h))
        }
        MediaFormat::Bmp if data.len() >= 26 && data.starts_with(b"BM") => {
            let w = i32::from_le_bytes(data[18..22].try_into().ok()?).unsigned_abs();
            let h = i32::from_le_bytes(data[22..26].try_into().ok()?).unsigned_abs();
            (w > 0 && h > 0).then_some((w, h))
        }
        _ => None,
    }
}

fn resolve_image_size(data: &[u8], format: MediaFormat, explicit_w: Option<Emu>, explicit_h: Option<Emu>) -> EmuSize {
    match (explicit_w, explicit_h) {
        (Some(w), Some(h)) => EmuSize::new(w, h),
        (Some(w), None) => {
            let h = infer_image_dimensions_px(data, format)
                .map(|(pw, ph)| ((w as f64) * (ph as f64) / (pw as f64)).round() as Emu)
                .unwrap_or(w * 2 / 3);
            EmuSize::new(w, h.max(EMU_PER_PT * 10))
        }
        (None, Some(h)) => {
            let w = infer_image_dimensions_px(data, format)
                .map(|(pw, ph)| ((h as f64) * (pw as f64) / (ph as f64)).round() as Emu)
                .unwrap_or(h * 3 / 2);
            EmuSize::new(w.max(EMU_PER_PT * 10), h)
        }
        (None, None) => {
            if let Some((pw, ph)) = infer_image_dimensions_px(data, format) {
                // Assume 96 DPI (9525 EMU per px), capped to 5 inches wide.
                let max_w = 5 * EMU_PER_INCH;
                let mut w = pw as Emu * 9_525;
                let mut h = ph as Emu * 9_525;
                if w > max_w {
                    h = ((h as f64) * (max_w as f64) / (w as f64)).round() as Emu;
                    w = max_w;
                }
                EmuSize::new(w.max(EMU_PER_PT * 10), h.max(EMU_PER_PT * 10))
            } else {
                EmuSize::new(3 * EMU_PER_INCH, 2 * EMU_PER_INCH)
            }
        }
    }
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let val = |b: u8| -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);
    let mut chunks = cleaned.chunks(4);
    for chunk in &mut chunks {
        match chunk.len() {
            4 => {
                let (a, b, c, d) = (val(chunk[0])?, val(chunk[1])?, val(chunk[2])?, val(chunk[3])?);
                out.push((a << 2) | (b >> 4));
                out.push((b << 4) | (c >> 2));
                out.push((c << 6) | d);
            }
            3 => {
                let (a, b, c) = (val(chunk[0])?, val(chunk[1])?, val(chunk[2])?);
                out.push((a << 2) | (b >> 4));
                out.push((b << 4) | (c >> 2));
            }
            2 => {
                let (a, b) = (val(chunk[0])?, val(chunk[1])?);
                out.push((a << 2) | (b >> 4));
            }
            _ => return None,
        }
    }
    Some(out)
}

fn encode_base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

// ============================================================================
// Serializer (FlowDocument -> Deterministic Rich Markdown)
// ============================================================================

/// Export a [`FlowDocument`] and its media to deterministic Rich Markdown.
pub fn export_markdown(doc: &FlowDocument, media: &[Media]) -> String {
    export_markdown_with_positions(doc, media, |_, _, _| {})
}

enum MarkdownSource<'a> {
    Body(usize, &'a Block),
    Note { note: &'a Note, endnote: bool },
    TableCell { index: usize, table: &'a FlowTable, row: usize, column: usize, cell: &'a FlowCell },
}

fn export_markdown_with_positions<'a>(
    doc: &'a FlowDocument,
    media: &[Media],
    mut record: impl FnMut(MarkdownSource<'a>, usize, usize),
) -> String {
    let mut out = String::new();

    // 1. YAML Frontmatter (written when metadata, non-default page setup, headers/footers, or watermark exist)
    let fm = serialize_frontmatter(doc);
    if !fm.is_empty() {
        out.push_str("---\n");
        out.push_str(&fm);
        out.push_str("---\n\n");
    }

    for xml in &doc.bibliography_sources {
        write_named_fence(BIBLIOGRAPHY_FENCE, xml, &mut out);
    }

    let comments_by_id: HashMap<u32, &Comment> = doc.comments.iter().map(|c| (c.id, c)).collect();

    // 2. Body blocks
    let mut prev_was_list = false;
    let mut has_sections = false;
    let form_layout = document_has_form_fields(doc);
    let mut i = 0;
    while i < doc.body.len() {
        let block = &doc.body[i];

        // A bibliography result can span several paragraphs. Only consume
        // a complete field so a malformed document cannot hide later text.
        if let Block::Paragraph(paragraph) = block
            && let Some(marker) = paragraph.field_start.as_deref().and_then(format_markdown_bibliography)
            && let Some(end) = generated_bibliography_end(&doc.body, i)
        {
            if prev_was_list && !out.ends_with("\n\n") {
                out.push('\n');
            }
            let start = out.len();
            out.push_str(&marker);
            out.push_str("\n\n");
            record(MarkdownSource::Body(i, block), start, out.len());
            prev_was_list = false;
            i = end + 1;
            continue;
        }

        // Collapse TOC field blocks back to `[[toc]]`
        if let Block::Paragraph(p) = block {
            if p.field_start
                .as_deref()
                .is_some_and(|f| f.trim_start().starts_with("TOC"))
            {
                if prev_was_list {
                    out.push('\n');
                    prev_was_list = false;
                }
                let start = out.len();
                out.push_str("[[toc]]\n\n");
                record(MarkdownSource::Body(i, block), start, out.len());
                while i < doc.body.len() {
                    let is_end = matches!(&doc.body[i], Block::Paragraph(ep) if ep.field_end);
                    i += 1;
                    if is_end {
                        break;
                    }
                }
                continue;
            }
        }

        let is_list = matches!(block, Block::Paragraph(p) if p.list.is_some() || p.runs.first().is_some_and(|r| matches!(r.kind, FlowRunKind::CheckBox { .. })));
        if prev_was_list && !is_list && !out.ends_with("\n\n") {
            out.push('\n');
        }

        let start = out.len();
        match block {
            Block::Table(table) => serialize_table(table, media, &comments_by_id, &mut out, &mut |row, column, cell, start, end| {
                record(MarkdownSource::TableCell { index: i, table, row, column, cell }, start, end);
            }),
            Block::Paragraph(p) => serialize_paragraph(p, doc, media, &comments_by_id, &mut out, form_layout),
        }
        record(MarkdownSource::Body(i, block), start, out.len());
        prev_was_list = is_list;
        if let Block::Paragraph(paragraph) = block
            && let Some(section) = &paragraph.section
        {
            write_section_marker(&mut out, paragraph.section_break, &section.page, section.restart_numbering,
                section.title_page, section.parts.each_ref().map(Option::as_deref));
            has_sections = true;
            prev_was_list = false;
        }
        i += 1;
    }

    let parts = [
        &doc.header, &doc.footer, &doc.header_first, &doc.footer_first,
        &doc.header_even, &doc.footer_even,
    ];
    if has_sections || doc.title_page || parts[2..].iter().any(|blocks| !blocks.is_empty()) {
        let parts = std::array::from_fn(|index| (!doc.linked[index]).then_some(parts[index].as_slice()));
        write_section_marker(&mut out, ibis_doc_model::SectionBreak::None, &doc.page, doc.restart_numbering,
            doc.title_page, parts);
    }

    // 3. Footnotes and endnotes have independent OOXML ID spaces.
    if !(doc.footnotes.is_empty() && doc.endnotes.is_empty()) && !out.ends_with("\n\n") && !out.is_empty() {
        out.push('\n');
    }
    for (notes, endnote) in [(&doc.footnotes, false), (&doc.endnotes, true)] {
        for note in notes {
            let start = out.len();
            let label = markdown_note_label(note.id, endnote);
            let mut blocks = note.blocks.clone();
            if let Some(Block::Paragraph(paragraph)) = blocks.first_mut() {
                let marker_runs = paragraph.runs.iter().take_while(|run| {
                    matches!(run.kind, FlowRunKind::NoteMark)
                        || matches!(&run.kind, FlowRunKind::Text(text) if text == " ")
                }).count();
                paragraph.runs.drain(..marker_runs);
                if paragraph.runs.is_empty() && paragraph.list.is_none() && blocks.len() > 1 {
                    blocks.remove(0);
                }
            }

            if let [Block::Paragraph(paragraph)] = blocks.as_slice()
                && paragraph.list.is_none()
                && !paragraph.runs.iter().any(|run| matches!(run.kind, FlowRunKind::LineBreak | FlowRunKind::Math { display: true, .. }))
            {
                let text = serialize_runs(&paragraph.runs, media, &comments_by_id);
                if !text.contains('\n') {
                    out.push_str(&format!("[^{label}]: {}\n", text.trim()));
                    record(MarkdownSource::Note { note, endnote }, start, out.len());
                    continue;
                }
            }

            out.push_str(&format!("[^{label}]:\n"));
            let mut body = String::new();
            for block in &blocks {
                serialize_block(block, doc, media, &comments_by_id, &mut body);
            }
            for line in body.trim_end().lines() {
                if !line.is_empty() {
                    out.push_str("    ");
                    out.push_str(line);
                }
                out.push('\n');
            }
            record(MarkdownSource::Note { note, endnote }, start, out.len());
        }
    }

    let trimmed = out.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

fn source_sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}

/// Create a versioned AI export tied to the exact input snapshot.
pub fn export_ai_source(data: &[u8]) -> Result<serde_json::Value, Error> {
    let mut result = if let Some(law) = crate::lawtext::export_ai_law(data)? {
        law
    } else {
        let presentation = crate::read(data)?;
        if presentation.workbook.is_some() {
            export_ai_workbook(&presentation)?
        } else {
            export_ai_document(&presentation)?
        }
    };
    result["sourceSha256"] = serde_json::json!(source_sha256(data));
    Ok(result)
}

/// Resolve an AI citation against a fresh export of the exact original input.
pub fn locate_ai_source(data: &[u8], query: &serde_json::Value) -> Result<serde_json::Value, Error> {
    if query["version"].as_u64() != Some(1) {
        return Err(Error::new("AI citation version must be 1"));
    }
    let expected = query["sourceSha256"].as_str().ok_or_else(|| Error::new("AI citation requires sourceSha256"))?;
    if source_sha256(data) != expected {
        return Err(Error::new("Source SHA-256 differs from the AI citation"));
    }
    let export = export_ai_source(data)?;
    locate_ai_export(&export, query)
}

fn locate_ai_export(export: &serde_json::Value, query: &serde_json::Value) -> Result<serde_json::Value, Error> {
    use serde_json::json;

    if query["version"].as_u64() != Some(1) {
        return Err(Error::new("AI citation version must be 1"));
    }
    let expected = query["sourceSha256"].as_str().ok_or_else(|| Error::new("AI citation requires sourceSha256"))?;
    if export["sourceSha256"] != expected {
        return Err(Error::new("Source SHA-256 differs from the AI citation"));
    }
    let (start, end) = ai_byte_range(&query["markdownBytes"]).ok_or_else(|| Error::new("AI citation requires a nonempty markdownBytes range"))?;
    let quote = query["quote"].as_str().ok_or_else(|| Error::new("AI citation requires the exact Markdown quote"))?;
    let markdown = export["markdown"].as_str().ok_or_else(|| Error::new("AI export has no Markdown"))?;
    if markdown.get(start..end) != Some(quote) {
        return Err(Error::new("Markdown range and quote do not match the regenerated export"));
    }
    let matches: Vec<_> = ["blocks", "articles", "cells"].into_iter()
        .filter_map(|key| export[key].as_array())
        .flatten()
        .filter(|entry| ai_byte_range(&entry["markdownBytes"])
            .is_some_and(|(left, right)| start < right && left < end))
        .collect();
    let mut result = json!({
        "version": 1,
        "kind": export["kind"],
        "sourceSha256": expected,
        "markdownBytes": [start, end],
        "quote": quote,
        "matches": matches,
    });
    if let Some(cells) = export["tableCells"].as_array() {
        let cell_matches: Vec<_> = cells.iter()
            .filter(|entry| ai_byte_range(&entry["markdownBytes"])
                .is_some_and(|(left, right)| start < right && left < end))
            .collect();
        result["tableCellMatches"] = json!(cell_matches);
    }
    Ok(result)
}

/// Locate the paragraph to reveal for a verified desktop citation.
pub fn locate_ai_paragraph(data: &[u8], query: &serde_json::Value) -> Result<u32, Error> {
    let citation = locate_ai_source(data, query)?;
    let matches = citation["matches"].as_array().map(Vec::as_slice).unwrap_or_default();
    let paragraph_id = match matches {
        [entry] if entry["source"]["part"] == "body" && entry["source"]["kind"] == "paragraph" => {
            &entry["source"]["modelId"]
        }
        [entry] if entry["source"]["part"] == "body" && entry["source"]["kind"] == "table" => {
            let cells = citation["tableCellMatches"].as_array().map(Vec::as_slice).unwrap_or_default();
            let [cell] = cells else {
                return Err(Error::new("The AI citation must identify one table cell"));
            };
            let ranges = ai_byte_range(&citation["markdownBytes"]).zip(ai_byte_range(&cell["markdownBytes"]));
            if !ranges.is_some_and(|((start, end), (left, right))| left <= start && end <= right) {
                return Err(Error::new("The AI citation extends outside the table cell"));
            }
            &cell["source"]["paragraphModelId"]
        }
        _ => return Err(Error::new("The AI citation must identify one body paragraph or table cell")),
    };
    paragraph_id.as_u64().and_then(|id| u32::try_from(id).ok())
        .ok_or_else(|| Error::new("The AI citation has no paragraph ID"))
}

fn ai_body_paragraph_entry(citation: &serde_json::Value) -> Result<&serde_json::Value, Error> {
    let matches = citation["matches"].as_array().map(Vec::as_slice).unwrap_or_default();
    let [entry] = matches else {
        return Err(Error::new("The AI citation must identify one body paragraph"));
    };
    let source = &entry["source"];
    if source["part"] != "body" || source["kind"] != "paragraph" {
        return Err(Error::new("The AI citation does not point to a body paragraph"));
    }
    Ok(entry)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum AiEditTarget {
    Paragraph(usize),
    TableCell { table: usize, row: usize, cell: usize },
}

struct PreparedAiEdit {
    target: AiEditTarget,
    run_index: usize,
    range: std::ops::Range<usize>,
    paragraph_start: usize,
    replacement: String,
    preview: serde_json::Value,
}

fn ai_edit_preview_digest(proposal: &serde_json::Value, edits: &[PreparedAiEdit]) -> String {
    let entries: Vec<_> = edits.iter().map(|edit| {
        let preview = &edit.preview;
        let source = &preview["source"];
        serde_json::json!([
            source["kind"], source["index"], source["modelId"],
            preview["operation"], preview["markdownBytes"], preview["quote"],
            preview["affectedParagraphBytes"], preview["before"], preview["replacement"],
        ])
    }).collect();
    let snapshot = serde_json::json!([
        "ibisOffice DOCX edit preview v1",
        proposal["sourceSha256"],
        entries,
    ]);
    source_sha256(snapshot.to_string().as_bytes())
}

/// Validate a single edit or a batch and return before/after text.
pub fn preview_ai_edit(data: &[u8], proposal: &serde_json::Value) -> Result<serde_json::Value, Error> {
    let (_, mut edits) = prepare_ai_edits(data, proposal)?;
    let digest = ai_edit_preview_digest(proposal, &edits);
    if proposal.get("edits").is_none() {
        let mut preview = edits.remove(0).preview;
        preview["previewSha256"] = digest.into();
        return Ok(preview);
    }
    let mut target_indices = HashMap::new();
    let mut target_edits: Vec<Vec<(usize, &PreparedAiEdit)>> = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        let target_index = *target_indices.entry(edit.target).or_insert_with(|| {
            target_edits.push(Vec::new());
            target_edits.len() - 1
        });
        target_edits[target_index].push((index, edit));
    }
    let targets: Vec<_> = target_edits.into_iter().map(|mut group| {
        let preview = &group[0].1.preview;
        let before = preview["before"].as_str().unwrap();
        let mut after = before.to_owned();
        let edit_indices: Vec<_> = group.iter().map(|(index, _)| index + 1).collect();
        group.sort_unstable_by_key(|(_, edit)| std::cmp::Reverse(edit.paragraph_start));
        for (_, edit) in group {
            let end = edit.paragraph_start + edit.range.len();
            after.replace_range(edit.paragraph_start..end, &edit.replacement);
        }
        serde_json::json!({
            "source": preview["source"],
            "before": before,
            "after": after,
            "editIndices": edit_indices,
        })
    }).collect();
    Ok(serde_json::json!({
        "version": 1,
        "sourceSha256": proposal["sourceSha256"],
        "previewSha256": digest,
        "edits": edits.into_iter().map(|edit| edit.preview).collect::<Vec<_>>(),
        "targets": targets,
    }))
}

/// Validate every edit against the original snapshot and write one DOCX.
pub fn apply_ai_edit(data: &[u8], proposal: &serde_json::Value) -> Result<Vec<u8>, Error> {
    let (bytes, _) = apply_ai_edit_inner(data, proposal, false)?;
    Ok(bytes)
}

/// Apply an edit and describe the generated bytes. The caller publishes the
/// receipt only after it has successfully saved those bytes.
pub fn apply_ai_edit_with_receipt(
    data: &[u8],
    proposal: &serde_json::Value,
) -> Result<(Vec<u8>, serde_json::Value), Error> {
    let (bytes, receipt) = apply_ai_edit_inner(data, proposal, true)?;
    Ok((bytes, receipt.unwrap()))
}

fn apply_ai_edit_inner(
    data: &[u8],
    proposal: &serde_json::Value,
    receipt: bool,
) -> Result<(Vec<u8>, Option<serde_json::Value>), Error> {
    let (mut opened, mut edits) = prepare_ai_edits(data, proposal)?;
    let expected = proposal.get("previewSha256");
    let digest = (receipt || expected.is_some()).then(|| ai_edit_preview_digest(proposal, &edits));
    if let Some(expected) = expected
        && expected.as_str() != digest.as_deref()
    {
        return Err(Error::new("AI edit differs from the checked preview SHA-256"));
    }
    // Each range refers to the original run. Changes to its right do not
    // shift the offsets of changes to its left.
    let edit_count = edits.len();
    edits.sort_unstable_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let doc = std::sync::Arc::make_mut(opened.presentation.flow.as_mut().unwrap());
    for PreparedAiEdit { target, run_index, range, replacement, .. } in edits {
        let block = match target {
            AiEditTarget::Paragraph(index) => &mut doc.body[index],
            AiEditTarget::TableCell { table, row, cell } => {
                let Block::Table(table) = &mut doc.body[table] else { unreachable!() };
                &mut table.rows[row].cells[cell].blocks[0]
            }
        };
        let Block::Paragraph(paragraph) = block else { unreachable!() };
        let FlowRunKind::Text(text) = &mut paragraph.runs[run_index].kind else { unreachable!() };
        text.replace_range(range, &replacement);
    }
    let bytes = opened.source.save(&opened.presentation, SaveOptions::default())?;
    let report = receipt.then(|| serde_json::json!({
        "version": 1,
        "sourceSha256": proposal["sourceSha256"],
        "previewSha256": digest,
        "previewVerified": expected.is_some(),
        "outputSha256": source_sha256(&bytes),
        "editCount": edit_count,
    }));
    Ok((bytes, report))
}

fn prepare_ai_edits(data: &[u8], proposal: &serde_json::Value) -> Result<(Opened, Vec<PreparedAiEdit>), Error> {
    let batch = proposal.get("edits");
    let proposals = match batch {
        Some(edits) => edits.as_array().filter(|edits| !edits.is_empty())
            .map(Vec::as_slice).ok_or_else(|| Error::new("AI edit batch requires a nonempty edits array"))?,
        None => std::slice::from_ref(proposal),
    };
    if proposal["version"].as_u64() != Some(1) || proposal["sourceSha256"] != source_sha256(data) {
        return Err(Error::new("AI edit requires version 1 and the original source SHA-256"));
    }
    let export = export_ai_source(data)?;
    let opened = crate::open(data.to_vec())?;
    if opened.source.has_package_signatures().map_err(|error| Error::new(error.to_string()))? {
        return Err(Error::new("AI editing does not support signed documents"));
    }
    let doc = opened.presentation.flow.as_ref().ok_or_else(|| Error::new("AI editing requires a DOCX document"))?;
    if doc.protection != ibis_doc_model::flow::DocumentProtection::None || doc.formatting_protection || doc.track_changes {
        return Err(Error::new("AI editing does not support protected documents or active change tracking"));
    }

    let mut ranges: HashMap<_, BTreeMap<usize, usize>> = HashMap::new();
    let mut edits = Vec::with_capacity(proposals.len());
    for (index, edit) in proposals.iter().enumerate() {
        let prepared = if batch.is_some() {
            let mut edit = edit.clone();
            let entry = edit.as_object_mut().ok_or_else(|| Error::new(format!("Edit {} must be an object", index + 1)))?;
            if entry.contains_key("previewSha256") {
                return Err(Error::new("previewSha256 belongs on the batch, not on an individual edit"));
            }
            for key in ["version", "sourceSha256"] {
                if entry.get(key).is_some_and(|value| value != &proposal[key]) {
                    return Err(Error::new(format!("Edit {} conflicts with the batch {key}", index + 1)));
                }
                entry.insert(key.into(), proposal[key].clone());
            }
            prepare_ai_edit(&opened, doc, &export, &edit)
                .map_err(|error| Error::new(format!("Edit {}: {error}", index + 1)))?
        } else {
            prepare_ai_edit(&opened, doc, &export, edit)?
        };
        let ranges = ranges.entry(prepared.target).or_default();
        let start = prepared.paragraph_start;
        let end = start + prepared.range.len();
        let overlaps_previous = ranges.range(..start).next_back().is_some_and(|(_, &right)| {
            right > start || (start == end && right == start)
        });
        let overlaps_next = ranges.range(start..=end).next().is_some_and(|(&left, &right)| {
            left < end || left == right || start == end
        });
        if overlaps_previous || overlaps_next {
            return Err(Error::new(format!("Edit {} overlaps or shares an insertion boundary with an earlier edit", index + 1)));
        }
        ranges.insert(start, end);
        edits.push(prepared);
    }
    Ok((opened, edits))
}

fn prepare_ai_edit(
    opened: &Opened,
    doc: &FlowDocument,
    export: &serde_json::Value,
    proposal: &serde_json::Value,
) -> Result<PreparedAiEdit, Error> {
    use serde_json::json;

    let citation = locate_ai_export(export, proposal)?;
    let cell_edit = match proposal.get("target") {
        None => false,
        Some(target) if target == "tableCell" => true,
        Some(_) => return Err(Error::new("Unsupported AI edit target")),
    };
    let entry = if cell_edit {
        let cells = citation["tableCellMatches"].as_array().map(Vec::as_slice).unwrap_or_default();
        let [entry] = cells else {
            return Err(Error::new("AI cell editing requires exactly one cited table cell"));
        };
        if proposal.get("paragraphBytes").is_none() && entry["markdownBytes"] != citation["markdownBytes"] {
            return Err(Error::new("AI cell editing requires the entire cell Markdown range"));
        }
        entry
    } else {
        ai_body_paragraph_entry(&citation)?
    };
    let before = proposal["before"].as_str().ok_or_else(|| Error::new("AI edit requires the original paragraph text in before"))?;
    let replacement = proposal["replacement"].as_str()
        .filter(|text| !text.chars().any(|ch| ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}')))
        .ok_or_else(|| Error::new("AI replacement must be a single-line string"))?;
    if replacement.is_empty() && proposal.get("paragraphBytes").is_none() {
        return Err(Error::new("AI deletion requires an explicit paragraphBytes range"));
    }
    let insert_before = match proposal.get("insert") {
        None => None,
        Some(value) => match value.as_str() {
            Some("before") => Some(true),
            Some("after") => Some(false),
            _ => return Err(Error::new("AI insert must be before or after")),
        },
    };
    if insert_before.is_some() && (proposal.get("paragraphBytes").is_none() || replacement.is_empty()) {
        return Err(Error::new("AI insertion requires an explicit paragraphBytes range and nonempty text"));
    }
    let source = &entry["source"];
    let index = source["index"].as_u64().and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| Error::new("The AI citation has no body block index"))?;
    let (target, paragraph) = match doc.body.get(index) {
        Some(Block::Paragraph(paragraph)) if !cell_edit && source["modelId"] == paragraph.id => {
            (AiEditTarget::Paragraph(index), paragraph)
        }
        Some(Block::Table(table)) if cell_edit && source["tableModelId"] == table.id => {
            let row = source["row"].as_u64().and_then(|row| row.checked_sub(1))
                .and_then(|row| usize::try_from(row).ok())
                .ok_or_else(|| Error::new("The AI citation has no row index"))?;
            let (cell_index, cell) = table.rows.get(row).and_then(|row| {
                row.cells.iter().enumerate().find(|(_, cell)| source["modelId"] == cell.id)
            }).ok_or_else(|| Error::new("AI edit cell was not found in the original document"))?;
            let [Block::Paragraph(paragraph)] = cell.blocks.as_slice() else {
                return Err(Error::new("AI cell editing requires one paragraph in the cell"));
            };
            if cell.span != 1 || cell.merge != VerticalMerge::None {
                return Err(Error::new("AI cell editing requires an unmerged cell"));
            }
            (AiEditTarget::TableCell { table: index, row, cell: cell_index }, paragraph)
        }
        _ => return Err(Error::new("AI edit target was not found in the original document")),
    };
    if paragraph.runs.iter().any(|run| run.link.is_some() || run.control.is_some() || run.revision.is_some())
        || paragraph.mark_revision.is_some() || !paragraph.bookmarks.is_empty()
        || !paragraph.bookmark_ends.is_empty() || paragraph.field_start.is_some()
        || paragraph.field_end || paragraph.subdoc.is_some()
    {
        return Err(Error::new("AI editing does not support this paragraph's annotations"));
    }
    let mut plain = String::new();
    let mut runs = Vec::with_capacity(paragraph.runs.len());
    for run in &paragraph.runs {
        let FlowRunKind::Text(text) = &run.kind else {
            return Err(Error::new("AI editing requires a paragraph containing only text runs"));
        };
        runs.push((plain.len(), text.as_str()));
        plain.push_str(text);
    }
    if plain != before {
        return Err(Error::new("Original paragraph text does not match before"));
    }
    let (run_index, range, paragraph_start, after) = if let Some(range) = proposal.get("paragraphBytes") {
        let (start, end) = ai_byte_range(range)
            .ok_or_else(|| Error::new("AI edit requires a nonempty paragraphBytes range"))?;
        let quote = citation["quote"].as_str().unwrap();
        let selected = before.get(start..end)
            .ok_or_else(|| Error::new("paragraphBytes must be within the original UTF-8 text"))?;
        let matches_quote = if cell_edit {
            escape_table_paragraph(selected.to_owned()) == quote
        } else {
            selected == quote
        };
        if !matches_quote {
            return Err(Error::new("paragraphBytes does not identify the exact cited text"));
        }
        let (run_index, &(run_start, text)) = runs.iter().enumerate()
            .find(|(_, (offset, text))| *offset <= start && end <= *offset + text.len())
            .ok_or_else(|| Error::new("AI replacement must stay within one text run"))?;
        let local = start - run_start..end - run_start;

        // Compare the actual Markdown positions by substituting a marker in a
        // temporary paragraph. This disambiguates repeated text and rejects
        // citations of generated markup, even when it matches literal text.
        let media = &opened.presentation.media;
        let comments = HashMap::new();
        let render = |paragraph: &FlowParagraph| {
            if cell_edit {
                return serialize_table_paragraph(paragraph, media, &comments);
            }
            let mut out = String::new();
            serialize_paragraph(paragraph, doc, media, &comments, &mut out, false);
            out
        };
        let original = render(paragraph);
        let mut marker = String::from("IbisAiEditMarker");
        while original.contains(&marker) {
            marker.push('X');
        }
        let mut marked = paragraph.clone();
        let mut marked_text = text.to_owned();
        marked_text.replace_range(local.clone(), &marker);
        marked.runs[run_index].kind = FlowRunKind::Text(marked_text.into());
        let rendered = render(&marked);
        let (block_start, _) = ai_byte_range(&entry["markdownBytes"]).unwrap();
        let (citation_start, citation_end) = ai_byte_range(&citation["markdownBytes"]).unwrap();
        let expected = citation_start.checked_sub(block_start).zip(citation_end.checked_sub(block_start))
            .and_then(|(left, right)| {
                (original.get(left..right) == Some(quote))
                    .then(|| format!("{}{}{}", &original[..left], marker, &original[right..]))
            });
        if expected.as_deref() != Some(&rendered) {
            return Err(Error::new("paragraphBytes and markdownBytes point to different text"));
        }

        let paragraph_range = match insert_before {
            Some(true) => start..start,
            Some(false) => end..end,
            None => start..end,
        };
        let local = paragraph_range.start - run_start..paragraph_range.end - run_start;
        let paragraph_start = paragraph_range.start;
        let mut after = plain;
        after.replace_range(paragraph_range, replacement);
        (run_index, local, paragraph_start, after)
    } else {
        if runs.len() != 1 {
            return Err(Error::new("Replacing a whole paragraph requires a single text run"));
        }
        (0, 0..plain.len(), 0, replacement.to_owned())
    };
    if before == after {
        return Err(Error::new("AI replacement does not change the paragraph"));
    }
    let operation = if insert_before.is_some() {
        "insert"
    } else if replacement.is_empty() {
        "delete"
    } else {
        "replace"
    };
    let preview = json!({
        "version": 1,
        "sourceSha256": proposal["sourceSha256"],
        "source": source,
        "operation": operation,
        "markdownBytes": citation["markdownBytes"],
        "quote": citation["quote"],
        "affectedParagraphBytes": [paragraph_start, paragraph_start + range.len()],
        "replacement": replacement,
        "before": before,
        "after": after,
    });
    Ok(PreparedAiEdit { target, run_index, range, paragraph_start, replacement: replacement.to_owned(), preview })
}

fn ai_byte_range(value: &serde_json::Value) -> Option<(usize, usize)> {
    let [left, right] = value.as_array()?.as_slice() else { return None };
    let left = usize::try_from(left.as_u64()?).ok()?;
    let right = usize::try_from(right.as_u64()?).ok()?;
    (left < right).then_some((left, right))
}

/// Known workbook omissions from Markdown export. Reports locations without cell contents.
pub fn workbook_markdown_omissions(p: &Presentation, options: WorkbookMarkdownOptions) -> Vec<serde_json::Value> {
    use ibis_doc_model::SheetState;
    use serde_json::json;

    let mut diagnostics = Vec::new();
    let Some(book) = &p.workbook else {
        return diagnostics;
    };
    let omitted_names = book.names.iter().filter(|name| {
        !options.include_defined_names || name.sheet.is_some_and(|index| {
            book.sheets.get(index).and_then(|sheet| workbook_markdown_sheet(p, sheet, options.include_hidden_sheets)).is_none()
        })
    }).count();
    if omitted_names > 0 {
        diagnostics.push(json!({"code": "defined_names_omitted", "count": omitted_names}));
    }
    for sheet_ref in &book.sheets {
        let name = &sheet_ref.name;
        if sheet_ref.other {
            diagnostics.push(json!({"code": "non_worksheet_omitted", "source": {"sheet": name}}));
            continue;
        }
        let Some(sheet) = workbook_markdown_sheet(p, sheet_ref, options.include_hidden_sheets) else {
            if sheet_ref.state != SheetState::Visible {
                diagnostics.push(json!({"code": "hidden_sheet_omitted", "source": {"sheet": name}}));
            }
            continue;
        };

        if !options.include_hidden_axes {
            let (rows, columns) = workbook_hidden_axis_ranges(sheet);
            for (code, ranges) in [("hidden_rows_omitted", rows), ("hidden_columns_omitted", columns)] {
                if !ranges.is_empty() {
                    diagnostics.push(json!({"code": code, "source": {"sheet": name, "ranges": ranges}}));
                }
            }
        }

        if !options.include_validations && !sheet.validations.is_empty() {
            let ranges: Vec<_> = sheet.validations.iter().flat_map(|rule| &rule.ranges)
                .map(validation_range_label).collect();
            diagnostics.push(json!({
                "code": "data_validation_omitted",
                "source": {"sheet": name, "ranges": ranges},
            }));
        }
    }
    diagnostics
}

fn workbook_hidden_axis_ranges(sheet: &ibis_doc_model::Sheet) -> (Vec<String>, Vec<String>) {
    (
        hidden_axis_ranges(&sheet.hidden_rows, &sheet.rows, |row| (row + 1).to_string()),
        hidden_axis_ranges(&sheet.hidden_columns, &sheet.columns, crate::xls_formula::column_letters),
    )
}

fn hidden_axis_ranges(
    hidden: &std::collections::BTreeMap<u32, ibis_doc_model::Emu>,
    sizes: &[ibis_doc_model::Emu],
    label: impl Fn(u32) -> String,
) -> Vec<String> {
    let zero_sizes = sizes.iter().enumerate()
        .filter(|&(_, size)| *size == 0)
        .map(|(index, _)| index as u32);
    let mut indices: Vec<_> = hidden.keys().copied().chain(zero_sizes).collect();
    indices.sort_unstable();
    indices.dedup();

    let mut ranges = Vec::new();
    for run in indices.chunk_by(|left, right| *right - *left == 1) {
        let first = run[0];
        let last = run[run.len() - 1];
        ranges.push(format!("{}:{}", label(first), label(last)));
    }
    ranges
}

/// Markdown and original sheet coordinates, with display text and stored values kept separate.
pub fn export_ai_workbook(p: &Presentation) -> Result<serde_json::Value, Error> {
    use ibis_doc_model::CellValue;
    use serde_json::json;

    let mut cells = Vec::new();
    let markdown = export_workbook_markdown_with_positions(p, WorkbookMarkdownOptions::default(), |sheet, row, column, cell, start, end| {
        let (kind, value) = match cell.map(|cell| &cell.value) {
            None | Some(CellValue::Empty) => ("empty", json!(null)),
            Some(CellValue::Number(number)) if number.is_finite() => ("number", json!(number)),
            Some(CellValue::Number(number)) => ("nonfinite_number", json!(number.to_string())),
            Some(CellValue::Text(text)) => ("text", json!(text)),
            Some(CellValue::Bool(value)) => ("boolean", json!(value)),
            Some(CellValue::Error(error)) => ("error", json!(error.to_string())),
        };
        cells.push(json!({
            "source": { "sheet": sheet, "row": row + 1, "column": column + 1 },
            "markdownBytes": [start, end],
            "display": cell.map(|cell| cell.text.as_str()).unwrap_or(""),
            "valueType": kind,
            "value": value,
            "formula": cell.and_then(|cell| cell.formula.as_deref()),
        }));
    })?;
    let mut diagnostics = vec![
        json!({ "code": "visible_sheet_cells_only" }),
        json!({ "code": "first_exported_row_used_as_markdown_header" }),
    ];
    diagnostics.extend(workbook_markdown_omissions(p, WorkbookMarkdownOptions::default()));
    Ok(json!({
        "version": 1,
        "kind": "workbook",
        "markdown": markdown,
        "cells": cells,
        "diagnostics": diagnostics,
    }))
}

fn ai_table_structure(
    table: &FlowTable,
    media: &[Media],
    comments: &HashMap<u32, &Comment>,
) -> serde_json::Value {
    use serde_json::json;

    let rows: Vec<_> = table.rows.iter().enumerate().map(|(index, row)| {
        let mut column = 1usize;
        let cells: Vec<_> = row.cells.iter().map(|cell| {
            let blocks: Vec<_> = cell.blocks.iter().map(|block| match block {
                Block::Paragraph(p) => json!({
                    "kind": "paragraph",
                    "modelId": p.id,
                    "markdown": serialize_runs(&p.runs, media, comments),
                }),
                Block::Table(table) => ai_table_structure(table, media, comments),
            }).collect();
            let merge = match cell.merge {
                VerticalMerge::None => "none",
                VerticalMerge::Start => "start",
                VerticalMerge::Continue => "continue",
            };
            let span = cell.span.max(1);
            let value = json!({
                "modelId": cell.id,
                "column": column,
                "columnSpan": span,
                "verticalMerge": merge,
                "blocks": blocks,
            });
            column = column.saturating_add(span as usize);
            value
        }).collect();
        json!({ "modelId": row.id, "row": index + 1, "header": row.header, "cells": cells })
    }).collect();
    json!({
        "kind": "table",
        "modelId": table.id,
        "columnCount": table.columns.len(),
        "rows": rows,
    })
}

fn first_paragraph_in_blocks(blocks: &[Block]) -> Option<&FlowParagraph> {
    blocks.iter().find_map(|block| match block {
        Block::Paragraph(paragraph) => Some(paragraph),
        Block::Table(table) => table.rows.iter().flat_map(|row| &row.cells)
            .find_map(|cell| first_paragraph_in_blocks(&cell.blocks)),
    })
}

/// Markdown, table structure, and snapshot-relative locations for DOCX body blocks and notes.
pub fn export_ai_document(presentation: &Presentation) -> Result<serde_json::Value, Error> {
    use serde_json::json;

    let doc = presentation.flow.as_ref().ok_or_else(|| Error::new("AI document export requires a word-processing document"))?;
    let mut blocks = Vec::new();
    let mut diagnostics = Vec::new();
    let mut table_cells = Vec::new();
    let markdown = export_markdown_with_positions(doc, &presentation.media, |source, start, end| {
        if let MarkdownSource::TableCell { index, table, row, column, cell } = source {
            table_cells.push(json!({
                "source": {
                    "part": "body", "kind": "tableCell", "index": index, "tableModelId": table.id,
                    "modelId": cell.id, "row": row, "column": column,
                    "paragraphModelId": first_paragraph_in_blocks(&cell.blocks).map(|p| p.id),
                },
                "markdownBytes": [start, end],
            }));
            return;
        }
        let locator = match source {
            MarkdownSource::Body(index, Block::Paragraph(p)) =>
                json!({ "part": "body", "kind": "paragraph", "index": index, "modelId": p.id }),
            MarkdownSource::Body(index, Block::Table(t)) =>
                json!({ "part": "body", "kind": "table", "index": index, "modelId": t.id }),
            MarkdownSource::Note { note, endnote: false } =>
                json!({ "part": "footnotes", "kind": "footnote", "id": note.id }),
            MarkdownSource::Note { note, endnote: true } =>
                json!({ "part": "endnotes", "kind": "endnote", "id": note.id }),
            MarkdownSource::TableCell { .. } => unreachable!(),
        };
        blocks.push((locator.clone(), start, end));
        let mut warn = |code| diagnostics.push(json!({ "code": code, "source": locator }));
        match source {
            MarkdownSource::TableCell { .. } => unreachable!(),
            MarkdownSource::Body(_, Block::Table(table)) => {
                let columns = markdown_table_columns(table);
                if table.rows.iter().any(|row| {
                    row.cells.iter().map(|cell| cell.span.max(1) as usize).sum::<usize>() > columns
                }) {
                    warn("table_merge_not_preserved");
                }
            }
            MarkdownSource::Body(_, Block::Paragraph(p)) => {
                if p.runs.iter().any(|run| match &run.kind {
                    FlowRunKind::Shape(shape) => match &shape.kind {
                        ShapeKind::Picture(picture) => !presentation.media(picture.media).is_some_and(|m| m.format.is_image()),
                        ShapeKind::Chart(_) => p.runs.len() != 1,
                        _ => true,
                    },
                    _ => false,
                }) {
                    warn("drawing_shape_omitted");
                }
            }
            MarkdownSource::Note { note, endnote } => {
                if note.blocks.iter().any(|block| matches!(block, Block::Table(_))) {
                    warn(if endnote { "endnote_table_omitted" } else { "footnote_table_omitted" });
                }
            }
        }
    });
    // The Markdown exporter trims the document's final blank lines.
    let blocks: Vec<_> = blocks.into_iter().map(|(source, start, end)| {
        json!({ "source": source, "markdownBytes": [start.min(markdown.len()), end.min(markdown.len())] })
    }).collect();
    let comments = doc.comments.iter().map(|comment| (comment.id, comment)).collect();
    let tables: Vec<_> = doc.body.iter().enumerate().filter_map(|(index, block)| {
        let Block::Table(table) = block else { return None };
        Some(json!({
            "source": { "part": "body", "kind": "table", "index": index, "modelId": table.id },
            "structure": ai_table_structure(table, &presentation.media, &comments),
        }))
    }).collect();
    Ok(json!({
        "version": 1,
        "kind": "document",
        "markdown": markdown,
        "blocks": blocks,
        "tables": tables,
        "tableCells": table_cells,
        "diagnostics": diagnostics,
    }))
}

fn document_has_form_fields(doc: &FlowDocument) -> bool {
    ibis_doc_model::flow_command::all_paragraphs(doc).iter()
        .any(|paragraph| paragraph.runs.iter().any(|run| form_field_id(run).is_some()))
}

fn serialize_frontmatter(doc: &FlowDocument) -> String {
    let mut lines = Vec::new();
    if doc.even_and_odd {
        lines.push("even_and_odd_headers: true".to_owned());
    }
    let props: &DocProperties = &doc.properties;
    if let Some(title) = props.title.as_deref().filter(|s| !s.is_empty()) {
        lines.push(format!("title: {}", quote_yaml_if_needed(title)));
    }
    if let Some(subject) = props.subject.as_deref().filter(|s| !s.is_empty()) {
        lines.push(format!("subtitle: {}", quote_yaml_if_needed(subject)));
    }
    if let Some(creator) = props.creator.as_deref().filter(|s| !s.is_empty()) {
        lines.push(format!("author: {}", quote_yaml_if_needed(creator)));
    }
    if let Some(keywords) = props.keywords.as_deref().filter(|s| !s.is_empty()) {
        lines.push(format!("keywords: {}", quote_yaml_if_needed(keywords)));
    }
    if let Some(description) = props.description.as_deref().filter(|s| !s.is_empty()) {
        lines.push(format!(
            "description: {}",
            quote_yaml_if_needed(description)
        ));
    }
    if let Some(company) = props.company.as_deref().filter(|s| !s.is_empty()) {
        lines.push(format!("company: {}", quote_yaml_if_needed(company)));
    }

    if document_has_form_fields(doc) {
        lines.push(format!("page_grid: {}", doc.page.doc_grid_type.ooxml()));
        if let Some(pitch) = doc.page.line_pitch {
            lines.push(format!("page_line_pitch: {:.2}pt", pitch as f64 / 12700.0));
        }
        if let Some(space) = doc.page.char_space {
            lines.push(format!("page_char_space_4096pt: {}", (space as f64 * 4096.0 / 12700.0).round() as i64));
        }
        for (side, length) in [
            ("top", doc.page.top), ("right", doc.page.right),
            ("bottom", doc.page.bottom), ("left", doc.page.left),
        ] {
            lines.push(format!("margin_{side}: {:.2}pt", length as f64 / 12700.0));
        }
        if let Some(normal) = doc.styles.default_paragraph.as_ref().and_then(|id| doc.styles.styles.get(id)) {
            if let Some(font) = &normal.run.font {
                lines.push(format!("font: {}", quote_yaml_if_needed(font)));
            }
            if let Some(font) = &normal.run.east_asian_font {
                lines.push(format!("east_asian_font: {}", quote_yaml_if_needed(font)));
            }
            if let Some(size) = normal.run.size {
                lines.push(format!("font_size: {:.2}pt", f64::from(size) / 100.0));
            }
        }
    }

    let default_page = PageSetup::default();
    let is_landscape = doc.page.size.width > doc.page.size.height;
    let (short_side, long_side) = (
        doc.page.size.width.min(doc.page.size.height),
        doc.page.size.width.max(doc.page.size.height),
    );
    if (short_side, long_side) != (default_page.size.width, default_page.size.height) {
        if let Some(name) = paper_size_name(short_side, long_side) {
            lines.push(format!("size: {name}"));
        }
    }
    if is_landscape {
        lines.push("orientation: landscape".to_string());
    }
    if doc.page.columns.count > 1 {
        lines.push(format!("columns: {}", doc.page.columns.count));
        if doc.page.columns.separator {
            lines.push("column_separator: true".to_string());
        }
    }
    if doc.page.direction == CellDirection::TbRl {
        lines.push("direction: vertical".to_string());
    }
    if let Some(ln) = &doc.page.line_numbers {
        let mode = if ln.restart == LineNumberRestart::Continuous {
            "continuous"
        } else {
            "true"
        };
        lines.push(format!("line_numbers: {mode}"));
    }
    if let Some(preset) = doc.style_set_preset {
        let name = match preset {
            StyleSetPreset::Default => "default",
            StyleSetPreset::Casual => "casual",
            StyleSetPreset::Centered => "centered",
            StyleSetPreset::Formal => "formal",
            StyleSetPreset::Elegant => "elegant",
            StyleSetPreset::Modern => "modern",
            StyleSetPreset::Simple => "simple",
            StyleSetPreset::Minimalist => "minimalist",
        };
        lines.push(format!("style_set: {name}"));
    }
    if let Some(preset) = doc.paragraph_spacing_preset {
        let name = match preset {
            ParagraphSpacingPreset::Default => "default",
            ParagraphSpacingPreset::NoSpace => "no_space",
            ParagraphSpacingPreset::Compact => "compact",
            ParagraphSpacingPreset::Tight => "tight",
            ParagraphSpacingPreset::Open => "open",
            ParagraphSpacingPreset::Relaxed => "relaxed",
            ParagraphSpacingPreset::Double => "double",
        };
        lines.push(format!("paragraph_spacing: {name}"));
    }
    if !doc.linked[0] && !doc.header.is_empty() {
        let h_text = serialize_header_footer(&doc.header);
        if !h_text.is_empty() {
            lines.push(format!("header: {}", quote_yaml_if_needed(&h_text)));
        }
    }
    if !doc.linked[1] && !doc.footer.is_empty() {
        let f_text = serialize_header_footer(&doc.footer);
        if !f_text.is_empty() {
            lines.push(format!("footer: {}", quote_yaml_if_needed(&f_text)));
        }
    }
    if let Some(wm) = &doc.watermark {
        lines.push(format!("watermark: {}", quote_yaml_if_needed(&wm.text)));
    }
    if let Some(pc) = doc.page_color {
        lines.push(format!("page_color: \"#{:06X}\"", pc.0));
    }
    if doc.track_changes {
        lines.push("track_changes: true".to_string());
    }

    if lines.is_empty() {
        String::new()
    } else {
        let mut s = lines.join("\n");
        s.push('\n');
        s
    }
}

fn serialize_header_footer(blocks: &[Block]) -> String {
    let mut s = String::new();
    for b in blocks {
        if let Block::Paragraph(p) = b {
            for r in &p.runs {
                match &r.kind {
                    FlowRunKind::Text(t) => s.push_str(t),
                    FlowRunKind::Field { instruction, shown } => {
                        if r.field_locked {
                            s.push_str(shown);
                        } else if let Some(marker) = format_markdown_page_field(&ibis_doc_model::flow_command::field_words(instruction)) {
                            s.push_str(&marker);
                        }
                    }
                    FlowRunKind::PageRef { bookmark, shown } => {
                        s.push_str(&format_markdown_page_ref(bookmark, shown, r.field_locked));
                    }
                    FlowRunKind::PageNumber => s.push_str("{page}"),
                    FlowRunKind::FileName(_) => s.push_str("{filename}"),
                    _ => {}
                }
            }
        }
    }
    s
}

fn quote_yaml_if_needed(s: &str) -> String {
    if s.contains(':')
        || s.contains('#')
        || s.contains('{')
        || s.contains('}')
        || s.contains('[')
        || s.contains(']')
        || s.starts_with(' ')
        || s.ends_with(' ')
    {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn paper_size_name(short: Emu, long: Emu) -> Option<&'static str> {
    let near = |a: Emu, b: Emu| (a - b).abs() < 50_000;
    let mm = |v: f64| (v * EMU_PER_MM).round() as Emu;
    if near(short, mm(297.0)) && near(long, mm(420.0)) {
        Some("A3")
    } else if near(short, 11_906 * 635) && near(long, 16_838 * 635) {
        Some("A4")
    } else if near(short, mm(148.0)) && near(long, mm(210.0)) {
        Some("A5")
    } else if near(short, mm(182.0)) && near(long, mm(257.0)) {
        Some("B5")
    } else if near(short, 8 * EMU_PER_INCH + EMU_PER_INCH / 2) && near(long, 11 * EMU_PER_INCH) {
        Some("Letter")
    } else if near(short, 8 * EMU_PER_INCH + EMU_PER_INCH / 2) && near(long, 14 * EMU_PER_INCH) {
        Some("Legal")
    } else {
        None
    }
}

fn serialize_block(
    block: &Block,
    doc: &FlowDocument,
    media: &[Media],
    comments: &HashMap<u32, &Comment>,
    out: &mut String,
) {
    match block {
        Block::Paragraph(p) => serialize_paragraph(p, doc, media, comments, out, false),
        Block::Table(t) => serialize_table(t, media, comments, out, &mut |_, _, _, _, _| {}),
    }
}

fn serialize_paragraph(
    p: &FlowParagraph,
    doc: &FlowDocument,
    media: &[Media],
    comments: &HashMap<u32, &Comment>,
    out: &mut String,
    form_layout: bool,
) {
    // 1. Standalone PageBreak / ColumnBreak
    if p.runs.len() == 1 {
        match &p.runs[0].kind {
            FlowRunKind::PageBreak => {
                out.push_str("\\newpage\n\n");
                return;
            }
            FlowRunKind::ColumnBreak => {
                out.push_str("\\columnbreak\n\n");
                return;
            }
            FlowRunKind::Math { math, display: true } => {
                out.push_str("$$\n");
                out.push_str(&math.to_latex());
                out.push_str("\n$$");
                if let Some(bookmark) = p.bookmarks.iter().find(|name| is_equation_bookmark(name)) {
                    out.push_str(&format!(" {{#{bookmark}}}"));
                }
                out.push_str("\n\n");
                return;
            }
            FlowRunKind::Shape(shape) => {
                if let ShapeKind::Chart(chart) = &shape.kind {
                    serialize_chart(chart, out);
                    return;
                }
            }
            FlowRunKind::Picture { look, .. } if look.alt_text.title == "__mermaid__" => {
                write_mermaid_fence(&look.alt_text.description, out);
                return;
            }
            _ => {}
        }
    }

    // 2. Horizontal rule (empty paragraph with bottom border only)
    if p.runs.is_empty()
        && p.direct.borders[2].is_some_and(|b| b.is_some())
        && p.direct.borders[0].is_none()
        && p.direct.borders[3].is_none()
    {
        out.push_str("---\n\n");
        return;
    }

    // 3. Fenced code block (shaded + 4 borders + Consolas font)
    if p.direct.shading.is_some_and(|s| s.is_some())
        && p.direct.borders[0].is_some_and(|b| b.is_some())
        && p.direct.borders[2].is_some_and(|b| b.is_some())
    {
        let mut code = String::new();
        for r in &p.runs {
            match &r.kind {
                FlowRunKind::Text(t) => code.push_str(t),
                FlowRunKind::LineBreak => code.push('\n'),
                FlowRunKind::Tab => code.push('\t'),
                _ => {}
            }
        }
        if is_literal_mermaid_fence(&code) {
            out.push_str(&code);
            out.push_str("\n\n");
        } else {
            out.push_str("```\n");
            out.push_str(&code);
            out.push_str("\n```\n\n");
        }
        return;
    }

    // 4. Blockquote / Alert (left border only)
    if p.direct.borders[3].is_some_and(|b| b.is_some()) && p.direct.borders[0].is_none() {
        if let Some(first_run) = p.runs.first()
            && let FlowRunKind::Text(label) = &first_run.kind
            && matches!(
                label.as_str(),
                "NOTE" | "TIP" | "IMPORTANT" | "WARNING" | "CAUTION"
            )
            && first_run.format.bold == Some(true)
        {
            out.push_str(&format!("> [!{label}]\n"));
            let rest_runs: Vec<FlowRun> = p
                .runs
                .iter()
                .skip(1)
                .skip_while(|r| matches!(r.kind, FlowRunKind::LineBreak))
                .cloned()
                .collect();
            let body = serialize_runs(&rest_runs, media, comments);
            for line in body.lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
            return;
        }
        let body = serialize_runs(&p.runs, media, comments);
        for line in body.lines() {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
        return;
    }

    let attr_suffix = format_block_attrs(p, form_layout && !p.runs.is_empty() && p.list.is_none());
    let serialize_text = || {
        if form_layout {
            serialize_form_runs(p, media, comments, true)
        } else {
            serialize_runs(&p.runs, media, comments)
        }
    };

    // 5. Heading
    let heading_level = p
        .style
        .as_deref()
        .and_then(|s| {
            s.strip_prefix("Heading")
                .or_else(|| s.strip_prefix("heading "))
                .and_then(|n| n.trim().parse::<u8>().ok())
                .or_else(|| {
                    doc.styles
                        .styles
                        .get(s)
                        .and_then(|st| st.name.strip_prefix("heading "))
                        .and_then(|n| n.trim().parse::<u8>().ok())
                })
        });

    if let Some(lvl) = heading_level.filter(|&l| (1..=6).contains(&l)) {
        let hashes = "#".repeat(lvl as usize);
        let text = serialize_text();
        out.push_str(&format!("{hashes} {text}{attr_suffix}\n\n"));
        return;
    }

    // 6. Task list item (`FlowRunKind::CheckBox` at start of paragraph)
    if let Some(FlowRun {
        kind: FlowRunKind::CheckBox { checked, .. },
        ..
    }) = p.runs.first()
    {
        let rest_runs: Vec<FlowRun> = p.runs[1..].to_vec();
        let text = serialize_runs(&rest_runs, media, comments);
        let text = text.strip_prefix(' ').unwrap_or(&text);
        let mark = if *checked { "[x]" } else { "[ ]" };
        out.push_str(&format!("- {mark} {text}{attr_suffix}\n"));
        return;
    }

    // 7. Bullet or Ordered list item
    if let Some(ListRef { num_id, level }) = p.list {
        let indent = "  ".repeat(level as usize);
        let is_ordered = doc
            .styles
            .numbering
            .level(num_id, level)
            .is_some_and(|lvl| lvl.format != "bullet");
        let marker = if is_ordered { "1." } else { "-" };
        let text = serialize_text();
        out.push_str(&format!("{indent}{marker} {text}{attr_suffix}\n"));
        return;
    }

    // 8. Normal paragraph
    let text = serialize_text();
    if !text.is_empty() || !attr_suffix.is_empty() {
        out.push_str(&format!("{text}{attr_suffix}\n\n"));
    }
}

fn format_block_attrs(p: &FlowParagraph, form_layout: bool) -> String {
    let mut parts = Vec::new();
    if let Some(bm) = p.bookmarks.first().filter(|b| !b.starts_with("_Toc")) {
        parts.push(format!("#{bm}"));
    }
    if let Some(align) = p.direct.align {
        match align {
            FlowAlign::Center => parts.push("align=center".to_string()),
            FlowAlign::Right => parts.push("align=right".to_string()),
            FlowAlign::Justify => parts.push("align=justify".to_string()),
            FlowAlign::Distribute => parts.push("align=distribute".to_string()),
            FlowAlign::Left => {}
        }
    }
    if form_layout {
        parts.push(format!("docx-spacing={}", form_spacing_value(p)));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" {{{}}}", parts.join(" "))
    }
}

fn escape_table_paragraph(text: String) -> String {
    text.replace('\n', "<br>").replace('|', "\\|")
}

fn serialize_form_runs(
    paragraph: &FlowParagraph, media: &[Media], comments: &HashMap<u32, &Comment>, resolved: bool,
) -> String {
    let mut text = String::new();
    for run in &paragraph.runs {
        let mut run = run.clone();
        if resolved {
            run.format.size = Some(run.style.size);
            run.format.bold = Some(run.style.bold);
            run.format.italic = Some(run.style.italic);
        }
        let piece = serialize_runs(std::slice::from_ref(&run), media, comments);
        if let Some(size) = run.format.size.filter(|_| !piece.is_empty() && !piece.contains('\n')) {
            use std::fmt::Write;
            let _ = write!(text, "{FORM_RUN_SIZE}{size} -->{piece}{FORM_RUN_END}");
        } else {
            text.push_str(&piece);
        }
    }
    text
}

fn serialize_table_paragraph(p: &FlowParagraph, media: &[Media], comments: &HashMap<u32, &Comment>) -> String {
    escape_table_paragraph(serialize_runs(&p.runs, media, comments))
}

fn markdown_table_columns(table: &FlowTable) -> usize {
    table.rows.iter().map(|row| row.cells.len()).max().unwrap_or(0).max(table.columns.len())
}

fn serialize_table<'a>(
    t: &'a FlowTable,
    media: &[Media],
    comments: &HashMap<u32, &Comment>,
    out: &mut String,
    record_cell: &mut dyn FnMut(usize, usize, &'a FlowCell, usize, usize),
) {
    if t.rows.is_empty() {
        return;
    }
    let col_count = markdown_table_columns(t);
    if col_count == 0 {
        return;
    }

    let form_layout = t.columns.len() == col_count && t.columns.iter().all(|width| (635..=31680 * 635).contains(width))
        && t.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.blocks).any(|block| {
            matches!(block, Block::Paragraph(paragraph) if paragraph.runs.iter().any(|run| form_field_id(run).is_some()))
        });
    let anchor = t.rows.iter().flat_map(|row| &row.cells).next()
        .map_or(ibis_doc_model::CellAnchor::Top, |cell| cell.anchor);

    let mut write_row = |row_index: usize, row: &'a FlowRow, out: &mut String| {
        out.push_str("| ");
        if form_layout {
            use std::fmt::Write;
            if row_index == 1 {
                let widths = t.columns.iter().map(|width| (width / 635).to_string()).collect::<Vec<_>>().join(",");
                let align = match t.align { FlowAlign::Center => "center", FlowAlign::Right => "right", _ => "left" };
                let anchor = match anchor {
                    ibis_doc_model::CellAnchor::Top => "top",
                    ibis_doc_model::CellAnchor::Center => "center",
                    ibis_doc_model::CellAnchor::Bottom => "bottom",
                };
                let _ = write!(out, "<!-- docx-table:{widths};{align};{anchor};{} --> ", row.header);
            }
            match row.height {
                RowHeight::Auto => {}
                RowHeight::AtLeast(height) => { let _ = write!(out, "<!-- docx-row:min={} --> ", height / 635); }
                RowHeight::Exact(height) => { let _ = write!(out, "<!-- docx-row:exact={} --> ", height / 635); }
            }
        }

        let mut column = 0;
        for (index, cell) in row.cells.iter().enumerate() {
            let span = (cell.span.max(1) as usize).min(col_count - column - (row.cells.len() - index - 1));
            if form_layout && cell.anchor != anchor {
                let name = match cell.anchor {
                    ibis_doc_model::CellAnchor::Top => "top",
                    ibis_doc_model::CellAnchor::Center => "center",
                    ibis_doc_model::CellAnchor::Bottom => "bottom",
                };
                out.push_str(&format!("<!-- docx-anchor:{name} --> "));
            }
            let mut cell_parts = Vec::new();
            for block in &cell.blocks {
                match block {
                    Block::Paragraph(paragraph) if form_layout => {
                        let mut text = form_paragraph_comment(paragraph);
                        text.push_str(&serialize_form_runs(paragraph, media, comments, false));
                        cell_parts.push(escape_table_paragraph(text));
                    }
                    Block::Paragraph(paragraph) => cell_parts.push(serialize_table_paragraph(paragraph, media, comments)),
                    Block::Table(table) => {
                        let mut source = String::new();
                        serialize_table(table, media, comments, &mut source, &mut |_, _, _, _, _| {});
                        let payload = escape_table_paragraph(source.trim_end()
                            .replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;"));
                        if !payload.is_empty() {
                            cell_parts.push(format!("{NESTED_TABLE_START}{payload}{NESTED_TABLE_END}"));
                        }
                    }
                }
            }
            let start = out.len();
            out.push_str(&cell_parts.join("<br>"));
            if start != out.len() {
                record_cell(row_index, column + 1, cell, start, out.len());
            }
            if span > 1 {
                use std::fmt::Write;
                let _ = write!(out, " <!-- colspan:{span} -->");
            }
            match cell.merge {
                VerticalMerge::Start => out.push_str(" <!-- vmerge:start -->"),
                VerticalMerge::Continue => out.push_str(" <!-- vmerge:continue -->"),
                VerticalMerge::None => {}
            }
            for _ in 0..span {
                out.push_str(" |");
                column += 1;
                if column < col_count {
                    out.push(' ');
                }
            }
        }
        while column < col_count {
            out.push_str(" |");
            column += 1;
            if column < col_count {
                out.push(' ');
            }
        }
        out.push('\n');
    };

    write_row(1, &t.rows[0], out);

    // Alignment separator row
    let mut sep_cells = Vec::with_capacity(col_count);
    for (index, cell) in t.rows[0].cells.iter().enumerate() {
        let span = (cell.span.max(1) as usize).min(col_count - sep_cells.len() - (t.rows[0].cells.len() - index - 1));
        let align = match cell.blocks.first() {
            Some(Block::Paragraph(paragraph)) => paragraph.direct.align,
            _ => None,
        };
        let marker = match align {
            Some(FlowAlign::Center) => ":---:",
            Some(FlowAlign::Right) => "---:",
            Some(FlowAlign::Left) => ":---",
            _ => "---",
        };
        sep_cells.push(marker);
        sep_cells.resize(sep_cells.len() + span - 1, "---");
    }
    sep_cells.resize(col_count, "---");
    out.push_str(&format!("| {} |\n", sep_cells.join(" | ")));

    for (index, row) in t.rows.iter().enumerate().skip(1) {
        write_row(index + 1, row, out);
    }
    out.push('\n');
}

/// The Mermaid fence still describes `chart`: same title, categories, values, and chart kind.
fn mermaid_source_matches(chart: &Chart, source: &str) -> bool {
    let Some(crate::mermaid::MermaidOutput::Chart(spec)) = crate::mermaid::render_mermaid(source) else {
        return false;
    };
    let live_title = chart.title.as_ref().map(|t| t.text.trim()).filter(|s| !s.is_empty());
    let spec_title = spec.title.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if live_title != spec_title {
        return false;
    }
    let live: Vec<_> = chart.groups.iter().flat_map(|g| g.series.iter()).collect();
    if live.len() != spec.series.len() {
        return false;
    }
    let categories = live.first().map(|s| s.categories.as_slice()).unwrap_or(&[]);
    if categories != spec.categories.as_slice() {
        return false;
    }
    for (series, (_, values)) in live.iter().zip(&spec.series) {
        if series.values.len() != values.len() {
            return false;
        }
        for (live_v, spec_v) in series.values.iter().zip(values) {
            match (live_v, spec_v) {
                (None, None) => {}
                (Some(Number(a)), Some(Number(b))) if (a - b).abs() < 1e-6 => {}
                _ => return false,
            }
        }
    }
    if spec.y_range.is_some() && chart_value_range(chart) != spec.y_range {
        return false;
    }
    let pie = chart.groups.iter().any(|g| matches!(g.kind, ChartType::Pie { .. } | ChartType::Doughnut { .. }));
    let line_only = chart.groups.len() == 1 && matches!(chart.groups[0].kind, ChartType::Line);
    match spec.kind {
        NewChartKind::Pie | NewChartKind::Doughnut => pie,
        NewChartKind::Line => line_only,
        NewChartKind::ClusteredBar => chart.groups.iter().all(|group| {
            matches!(group.kind, ChartType::Bar { horizontal: true, .. })
        }),
        NewChartKind::ClusteredColumn => chart.groups.iter().any(|group| {
            matches!(group.kind, ChartType::Bar { horizontal: false, .. })
        }),
        _ => false,
    }
}

fn write_mermaid_fence(source: &str, out: &mut String) {
    write_named_fence("mermaid", source, out);
}

fn write_named_fence(language: &str, source: &str, out: &mut String) {
    let longest = source.lines().map(|line| {
        line.trim_start().bytes().take_while(|&byte| byte == b'`').count()
    }).max().unwrap_or(0);
    let marker = "`".repeat(3.max(longest + 1));
    out.push_str(&marker);
    out.push_str(language);
    out.push('\n');
    out.push_str(source);
    if !source.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&marker);
    out.push_str("\n\n");
}

fn serialize_chart(chart: &Chart, out: &mut String) {
    if let Some(source) = chart.mermaid_source.as_str().filter(|source| mermaid_source_matches(chart, source)) {
        write_mermaid_fence(source, out);
        return;
    }
    out.push_str("```chart\n");
    let kind_name = match chart.groups.first().map(|g| (&g.kind, g.grouping)) {
        Some((ChartType::Bar { horizontal: false, .. }, Grouping::Standard)) => "column",
        Some((ChartType::Bar { horizontal: false, .. }, Grouping::Stacked)) => "stacked-column",
        Some((ChartType::Bar { horizontal: false, .. }, Grouping::PercentStacked)) => {
            "percent-stacked-column"
        }
        Some((ChartType::Bar { horizontal: true, .. }, Grouping::Standard)) => "bar",
        Some((ChartType::Bar { horizontal: true, .. }, Grouping::Stacked)) => "stacked-bar",
        Some((ChartType::Bar { horizontal: true, .. }, Grouping::PercentStacked)) => {
            "percent-stacked-bar"
        }
        Some((ChartType::Line, _)) => "line",
        Some((ChartType::Area, Grouping::Stacked | Grouping::PercentStacked)) => "stacked-area",
        Some((ChartType::Area, Grouping::Standard)) => "area",
        Some((ChartType::Pie { .. }, _)) => "pie",
        Some((ChartType::Doughnut { .. }, _)) => "doughnut",
        Some((ChartType::Radar { .. }, _)) => "radar",
        Some((ChartType::Scatter { .. }, _)) => "scatter",
        _ => "column",
    };
    out.push_str(&format!("type: {kind_name}\n"));
    if let Some(title) = &chart.title
        && !title.text.is_empty()
    {
        out.push_str(&format!("title: {}\n", quote_yaml_if_needed(&title.text)));
    }
    let all_series: Vec<&Series> = chart.groups.iter().flat_map(|g| &g.series).collect();
    if let Some(first) = all_series.first()
        && !first.categories.is_empty()
    {
        let cats: Vec<String> = first
            .categories
            .iter()
            .map(|c| quote_yaml_if_needed(c))
            .collect();
        out.push_str(&format!("categories: [{}]\n", cats.join(", ")));
    }
    if !all_series.is_empty() {
        out.push_str("series:\n");
        for s in all_series {
            let vals: Vec<String> = s
                .values
                .iter()
                .map(|v| match v {
                    Some(Number(n)) => {
                        if (*n - n.round()).abs() < 1e-9 {
                            format!("{}", *n as i64)
                        } else {
                            format!("{n}")
                        }
                    }
                    None => "0".to_string(),
                })
                .collect();
            out.push_str(&format!(
                "  - name: {}, values: [{}]\n",
                quote_yaml_if_needed(&s.name),
                vals.join(", ")
            ));
        }
    }
    out.push_str("```\n\n");
}

fn markdown_note_label(id: u32, endnote: bool) -> String {
    if endnote {
        format!("endnote-{id}")
    } else {
        id.to_string()
    }
}

fn markdown_endnote_id(label: &str) -> Option<u32> {
    let id = label.strip_prefix("endnote-")?;
    if id.starts_with('0') || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    id.parse().ok()
}

fn serialize_picture(
    media: &[Media],
    id: MediaId,
    size: EmuSize,
    alt_text: &ibis_doc_model::AltText,
) -> String {
    let Some(media) = media.iter().find(|media| media.id == id) else {
        return String::new();
    };
    let mime = match media.format {
        MediaFormat::Png => "png",
        MediaFormat::Jpeg => "jpeg",
        MediaFormat::Gif => "gif",
        MediaFormat::Bmp => "bmp",
        MediaFormat::Tiff => "tiff",
        MediaFormat::Webp => "webp",
        MediaFormat::Svg => "svg+xml",
        MediaFormat::Emf => "x-emf",
        MediaFormat::Wmf => "x-wmf",
        _ => return String::new(),
    };
    let alt = if alt_text.description.is_empty() { &alt_text.title } else { &alt_text.description };
    let b64 = encode_base64(&media.data);
    let width = (size.width as f64 / EMU_PER_MM).round() as i64;
    let height = (size.height as f64 / EMU_PER_MM).round() as i64;
    format!("![{alt}](data:image/{mime};base64,{b64}){{width={width}mm height={height}mm}}")
}

fn serialize_runs(
    runs: &[FlowRun],
    media: &[Media],
    comments: &HashMap<u32, &Comment>,
) -> String {
    let mut out = String::new();
    for run in runs {
        let mut piece = match &run.kind {
            FlowRunKind::Text(t) if t.contains("{{form:")
                && !(run.format.font.as_deref() == Some("Consolas") && run.format.highlight.is_some_and(|h| h.is_some())) =>
            {
                format_text_run(&t.replace("{{form:", r"\{{form:"), &run.format, run.link.is_some())
            }
            FlowRunKind::Text(t) => format_text_run(t, &run.format, run.link.is_some()),
            FlowRunKind::PlainText { .. }
                if let Some(id) = form_field_id(run) =>
            {
                format_text_run(&format!("{{{{form:{id}}}}}"), &run.format, run.link.is_some())
            }
            FlowRunKind::Field { instruction, shown } => {
                if run.field_locked {
                    shown.clone()
                } else {
                    format_markdown_inline_field(instruction, shown)
                }
            }
            FlowRunKind::LineBreak => "  \n".to_string(),
            FlowRunKind::Tab => "\t".to_string(),
            FlowRunKind::PageBreak => "\\newpage".to_string(),
            FlowRunKind::ColumnBreak => "\\columnbreak".to_string(),
            FlowRunKind::PageNumber => "{{field:page}}".to_string(),
            FlowRunKind::PageRef { bookmark, shown } => format_markdown_page_ref(bookmark, shown, run.field_locked),
            FlowRunKind::FileName(_) => "{filename}".to_string(),
            FlowRunKind::Math { math, display } => {
                let l = math.to_latex();
                if *display {
                    format!("$${l}$$")
                } else {
                    format!("${l}$")
                }
            }
            FlowRunKind::Ruby { base, text, .. } => format!("{{{base}|{text}}}"),
            FlowRunKind::NoteRef { id, endnote } => format!("[^{}]", markdown_note_label(*id, *endnote)),
            FlowRunKind::CommentRef(cid) => {
                if let Some(c) = comments.get(cid) {
                    let body: String = c
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            Block::Paragraph(p) => Some(p.text()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    if c.author.is_empty() || c.author == "Author" {
                        format!("{{>>{body}<<}}")
                    } else {
                        format!("{{>>{}: {body}<<}}", c.author)
                    }
                } else {
                    String::new()
                }
            }
            FlowRunKind::Picture {
                media: mid,
                size,
                look,
            } => {
                serialize_picture(media, *mid, *size, &look.alt_text)
            }
            FlowRunKind::Shape(shape) => match &shape.kind {
                ShapeKind::Picture(picture) => serialize_picture(
                    media,
                    picture.media,
                    EmuSize::new(shape.frame.width, shape.frame.height),
                    &picture.alt_text,
                ),
                _ => String::new(),
            },
            _ => String::new(),
        };

        if let Some(link) = &run.link
            && !piece.is_empty()
        {
            let target = match link {
                Link::Url(u) => u.clone(),
                Link::Anchor(b) => format!("#{b}"),
            };
            piece = format!("[{piece}]({target})");
        }

        if let Some(rev) = &run.revision
            && !piece.is_empty()
        {
            match rev.kind {
                RevisionKind::Insert | RevisionKind::MoveTo => {
                    piece = format!("{{++{piece}++}}");
                }
                RevisionKind::Delete | RevisionKind::MoveFrom => {
                    piece = format!("{{--{piece}--}}");
                }
                _ => {}
            }
        }

        out.push_str(&piece);
    }
    out
}

fn format_text_run(text: &str, fmt: &RunFormat, is_link: bool) -> String {
    if text.is_empty() {
        return String::new();
    }
    // Inline code detection
    if fmt.font.as_deref() == Some("Consolas") && fmt.highlight.is_some_and(|h| h.is_some()) {
        return format!("`{text}`");
    }

    let mut s = text.to_string();
    if fmt.tate_chu_yoko == Some(true) {
        s = format!("[{s}]{{.tcy}}");
    }
    if fmt.emphasis.is_some() {
        s = format!("《《{s}》》");
    }
    match fmt.position {
        Some(VerticalPosition::Superscript) => s = format!("^{s}^"),
        Some(VerticalPosition::Subscript) => s = format!("~{s}~"),
        _ => {}
    }
    if fmt.highlight.is_some_and(|h| h.is_some()) {
        s = format!("=={s}==");
    }
    if fmt.underline == Some(true) && !is_link {
        s = format!("++{s}++");
    }
    if fmt.strike == Some(true) || fmt.double_strike == Some(true) {
        s = format!("~~{s}~~");
    }
    match (fmt.bold == Some(true), fmt.italic == Some(true)) {
        (true, true) => s = format!("***{s}***"),
        (true, false) => s = format!("**{s}**"),
        (false, true) => s = format!("*{s}*"),
        (false, false) => {}
    }
    s
}

// ============================================================================
// Presentation (PPTX) Markdown Import & Export
// ============================================================================

/// Open a Markdown document as a PowerPoint presentation (`.pptx` model).
///
/// Slides are split on thematic breaks (`---` / `***` / `___`) outside fenced code blocks,
/// or on top-level `# Heading` sections when no thematic break is present.
pub fn open_markdown_presentation(data: Vec<u8>) -> Result<Opened, Error> {
    open_markdown_presentation_with_base_dir(data, None)
}

const PPTX_SLIDE_MARKER: &str = "<!-- pptx-slide -->";
const PPTX_SHAPE_MARKER: &str = "<!-- pptx-shape:";

struct MarkdownShapePlacement {
    frame: EmuRect,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
    auto_shape: Option<ibis_doc_model::AutoShape>,
}

fn parse_markdown_shape_placement(text: &str) -> Result<MarkdownShapePlacement, Error> {
    use ibis_doc_model::{AutoShape, Color, Geometry};
    let invalid = || Error::new("pptx-shape の配置指定が不正です");
    let encoded = text.strip_prefix(PPTX_SHAPE_MARKER)
        .and_then(|text| text.strip_suffix(" -->")).ok_or_else(invalid)?;
    let data: serde_json::Value = serde_json::from_str(encoded).map_err(|_| invalid())?;
    let Some([x, y, width, height]) = data["frame"].as_array().map(Vec::as_slice) else {
        return Err(invalid());
    };
    const MAX_EMU: i64 = 27_273_042_316_900;
    let coord = |v: &serde_json::Value| v.as_i64().filter(|n| (-MAX_EMU..=MAX_EMU).contains(n)).ok_or_else(invalid);
    let extent = |v: &serde_json::Value| coord(v).and_then(|n| if n >= 0 { Ok(n) } else { Err(invalid()) });
    let frame = EmuRect::new(coord(x)?, coord(y)?, extent(width)?, extent(height)?);
    let rotation = data.get("rotation").map_or(Ok(0), |v| v.as_i64()
        .and_then(|n| i32::try_from(n).ok()).ok_or_else(invalid))?;
    let flip = |key| data.get(key).map_or(Ok(false), |v| v.as_bool().ok_or_else(invalid));
    let auto_shape = if let Some(preset) = data.get("preset") {
        let geometry = match preset.as_str() {
            Some("rect") => Geometry::Rect,
            Some("ellipse") => Geometry::Ellipse,
            Some("roundRect") => Geometry::RoundRect {
                radius: data.get("radius").map_or(Ok(16_667), |v| v.as_u64()
                    .filter(|n| *n <= 50_000).map(|n| n as u32).ok_or_else(invalid))?,
            },
            _ => return Err(invalid()),
        };
        let fill = if let Some(fill) = data.get("fill") {
            let Some([rgb, alpha]) = fill.as_array().map(Vec::as_slice) else { return Err(invalid()); };
            let rgb = rgb.as_u64().filter(|n| *n <= 0xffffff).ok_or_else(invalid)?;
            let alpha = alpha.as_u64().and_then(|n| u8::try_from(n).ok()).ok_or_else(invalid)?;
            Fill::Solid(Color { rgb: Rgb(rgb as u32), alpha })
        } else {
            Fill::None
        };
        Some(AutoShape::new(geometry, fill, None))
    } else {
        None
    };
    Ok(MarkdownShapePlacement { frame, rotation, flip_h: flip("flip_h")?, flip_v: flip("flip_v")?, auto_shape })
}

fn write_markdown_shape_placement(out: &mut String, shape: &ibis_doc_model::Shape) {
    use ibis_doc_model::Geometry;
    let EmuRect { x, y, width, height } = shape.frame;
    let mut data = serde_json::json!({ "frame": [x, y, width, height] });
    if shape.rotation != 0 {
        data["rotation"] = shape.rotation.into();
    }
    if shape.flip_h {
        data["flip_h"] = true.into();
    }
    if shape.flip_v {
        data["flip_v"] = true.into();
    }
    if let ShapeKind::Auto(auto) = &shape.kind {
        let preset = match auto.geometry {
            Geometry::Rect => Some("rect"),
            Geometry::Ellipse => Some("ellipse"),
            Geometry::RoundRect { radius } => {
                data["radius"] = radius.into();
                Some("roundRect")
            }
            Geometry::Formula(_) => None,
        };
        if let Some(preset) = preset {
            data["preset"] = preset.into();
            if let Fill::Solid(color) = &auto.fill {
                data["fill"] = serde_json::json!([color.rgb.0, color.alpha]);
            }
        }
    }
    out.push_str(PPTX_SHAPE_MARKER);
    out.push_str(&data.to_string());
    out.push_str(" -->\n\n");
}

/// Open a Markdown document as a PowerPoint presentation, resolving relative image paths against `base_dir`.
pub fn open_markdown_presentation_with_base_dir(
    data: Vec<u8>,
    base_dir: Option<&std::path::Path>,
) -> Result<Opened, Error> {
    let raw_text = String::from_utf8(data)
        .map_err(|_| Error::new("Markdown ファイルが有効な UTF-8 ではありません"))?;
    let slide_chunks = split_markdown_slides(&raw_text);
    let blank = crate::blank::blank_file(crate::Kind::Presentation)?;
    let mut opened = crate::save::open(blank)?;
    let trimmed = raw_text.trim_start_matches('\u{feff}').trim_start();
    if let Some((frontmatter, _)) = trimmed.strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n")).and_then(split_frontmatter_body)
        && let Some(size) = frontmatter.lines().find_map(|line| line.strip_prefix("pptx_size:"))
    {
        let [width, height]: [i64; 2] = serde_json::from_str(size)
            .map_err(|_| Error::new("pptx_size の指定が不正です"))?;
        if !(1..=i64::from(i32::MAX)).contains(&width) || !(1..=i64::from(i32::MAX)).contains(&height) {
            return Err(Error::new("pptx_size の指定が不正です"));
        }
        opened.presentation.slide_size = EmuSize::new(width, height);
    }

    let mut next_shape_id: u64 = 100;
    let mut fresh_shape_id = || {
        next_shape_id += 1;
        ShapeId(next_shape_id)
    };
    let mut next_slide_id: u64 = 256;
    let mut fresh_slide_id = || {
        next_slide_id += 1;
        ibis_doc_model::SlideId(next_slide_id)
    };

    let title_layout = opened
        .presentation
        .layouts
        .first()
        .map(|l| l.id)
        .ok_or_else(|| Error::new("プレゼンテーションレイアウトが見つかりません"))?;
    let content_layout = opened
        .presentation
        .layouts
        .get(1)
        .map(|l| l.id)
        .unwrap_or(title_layout);

    enum SlideBlock<'a> {
        Text(Vec<ibis_doc_model::Paragraph>),
        Table(&'a FlowTable),
        Picture(MediaId, EmuSize, &'a PictureLook),
        Chart(&'a Chart),
    }

    for (idx, chunk) in slide_chunks.iter().enumerate() {
        let positioned = chunk.starts_with(PPTX_SLIDE_MARKER);
        let chunk = chunk.strip_prefix(PPTX_SLIDE_MARKER).unwrap_or(chunk);
        let (clean_chunk, notes) = extract_slide_notes(chunk);
        let parsed = open_markdown_with_base_dir(clean_chunk.into_bytes(), base_dir)?;
        let Some(flow) = parsed.presentation.flow.as_ref() else {
            continue;
        };

        // Ensure slide `idx` exists as a writable `NewSlide` in `opened.presentation`.
        let layout_for_slide = if idx == 0 { title_layout } else { content_layout };
        let sid = fresh_slide_id();
        let new_slide = opened.source.new_slide(
            &opened.presentation,
            idx,
            sid,
            layout_for_slide,
            &mut fresh_shape_id,
        )?;
        let mut new_slide = new_slide;
        if positioned {
            new_slide.shapes.clear();
            new_slide.placeholders.clear();
        }
        if idx < opened.presentation.slides.len() {
            opened.presentation.slides[idx] = new_slide;
        } else {
            opened.presentation.slides.push(new_slide);
        }

        // Copy media from the parsed chunk into the presentation, remapping MediaId.
        let mut media_remap: HashMap<MediaId, MediaId> = HashMap::new();
        for m in &parsed.presentation.media {
            let new_mid = opened.presentation.add_media(m.format, m.data.to_vec());
            media_remap.insert(m.id, new_mid);
        }

        let mut slide_title: Option<String> = None;
        let mut body_blocks: Vec<&Block> = Vec::new();
        for b in &flow.body {
            if !positioned && slide_title.is_none() {
                if let Block::Paragraph(p) = b {
                    let is_heading = p
                        .style
                        .as_deref()
                        .is_some_and(|s| s.starts_with("Heading"));
                    if is_heading {
                        let t = flow_paragraph_plain_text(p);
                        if !t.trim().is_empty() {
                            slide_title = Some(t.trim().to_string());
                            continue;
                        }
                    }
                }
            }
            body_blocks.push(b);
        }

        // Keep source order in the shape list, which determines the stacking order.
        let mut blocks: Vec<(SlideBlock<'_>, Option<MarkdownShapePlacement>)> = Vec::new();
        for b in body_blocks {
            let mut visuals = Vec::new();
            match b {
                Block::Paragraph(p) => {
                    if positioned {
                        let text = flow_paragraph_plain_text(p);
                        if text.starts_with(PPTX_SHAPE_MARKER) {
                            blocks.push((SlideBlock::Text(Vec::new()), Some(parse_markdown_shape_placement(&text)?)));
                            continue;
                        }
                    }
                    for r in &p.runs {
                        match &r.kind {
                            FlowRunKind::Picture { media, size, look } => {
                                if let Some(&new_mid) = media_remap.get(media) {
                                    visuals.push(SlideBlock::Picture(new_mid, *size, look));
                                }
                            }
                            FlowRunKind::Shape(shape) => {
                                if let ShapeKind::Chart(chart) = &shape.kind {
                                    visuals.push(SlideBlock::Chart(chart));
                                }
                            }
                            _ => {}
                        }
                    }
                    if visuals.is_empty()
                        && let Some(paragraph) = flow_paragraph_to_slide_paragraph(p)
                    {
                        if let Some((SlideBlock::Text(paragraphs), _)) = blocks.last_mut() {
                            paragraphs.push(paragraph);
                        } else {
                            blocks.push((SlideBlock::Text(vec![paragraph]), None));
                        }
                    }
                }
                Block::Table(tbl) => visuals.push(SlideBlock::Table(tbl)),
            }
            for visual in visuals {
                match blocks.last_mut() {
                    Some((slot @ SlideBlock::Text(_), Some(placement)))
                        if matches!(&*slot, SlideBlock::Text(text) if text.is_empty())
                            && placement.auto_shape.is_none() => *slot = visual,
                    _ => blocks.push((visual, None)),
                }
            }
        }

        let slide_id = opened.presentation.slides[idx].id;
        let line_count = |paragraphs: &[ibis_doc_model::Paragraph]| -> i64 {
            paragraphs
                .iter()
                .map(|para| 1 + para.runs.iter().map(|run| run.text.matches('\n').count()).sum::<usize>())
                .sum::<usize>() as i64
        };
        let text_lines: i64 = blocks
            .iter()
            .filter_map(|(block, _)| match block {
                SlideBlock::Text(paragraphs) => Some(line_count(paragraphs)),
                _ => None,
            })
            .sum();
        let has_visuals = blocks.iter().any(|(block, _)| !matches!(block, SlideBlock::Text(_)));

        // Populate title placeholder (shape 0).
        if let Some(title_text) = slide_title {
            if let Some(sh) = opened.presentation.slides[idx].shapes.get_mut(0) {
                if let Some(body) = shape_text_body_mut(&mut sh.kind) {
                    body.paragraphs = vec![ibis_doc_model::Paragraph {
                        runs: vec![ibis_doc_model::Run {
                            text: title_text,
                            style: ibis_doc_model::RunStyle {
                                font_size: Some(if idx == 0 { 4000 } else { 2800 }),
                                bold: Some(true),
                                ..ibis_doc_model::RunStyle::default()
                            },
                        }],
                        ..ibis_doc_model::Paragraph::default()
                    }];
                }
            }
        }

        let text_width = if has_visuals { 5_100_000 } else { 10_668_000 };
        let visual_frame = if text_lines == 0 {
            EmuRect::new(762_000, 1_550_000, 10_668_000, 4_750_000)
        } else {
            EmuRect::new(6_160_000, 1_550_000, 5_270_000, 4_750_000)
        };
        let shapes = &mut opened.presentation.slides[idx].shapes;
        let mut body_placeholder = (shapes.len() > 1).then(|| shapes.remove(1));
        // Interleaved visuals split the text into multiple shapes. Share the original
        // text column between them, counting explicit line breaks in literal fences.
        let mut used_lines = 0;
        for (block, mut placement) in blocks {
            let before = opened.presentation.slides[idx].shapes.len();
            let visual_frame = placement.as_ref().map_or(visual_frame, |placement| placement.frame);
            match block {
                SlideBlock::Text(paragraphs) => {
                    let top = 4_750_000 * used_lines / text_lines.max(1);
                    used_lines += line_count(&paragraphs);
                    let bottom = 4_750_000 * used_lines / text_lines.max(1);
                    let frame = EmuRect::new(762_000, 1_550_000 + top, text_width, bottom - top);
                    let mut shape = body_placeholder.take().unwrap_or_else(|| ibis_doc_model::Shape {
                        id: fresh_shape_id(),
                        frame,
                        rotation: 0,
                        flip_h: false,
                        flip_v: false,
                        kind: ShapeKind::TextBox(Default::default()),
                        floating: None,
                    });
                    shape.frame = frame;
                    if let Some(mut auto) = placement.as_mut().and_then(|place| place.auto_shape.take()) {
                        if !paragraphs.is_empty() {
                            auto.text = Some(ibis_doc_model::TextBody { paragraphs, ..Default::default() });
                        }
                        shape.kind = ShapeKind::Auto(auto);
                    } else if let Some(body) = shape_text_body_mut(&mut shape.kind) {
                        body.paragraphs = paragraphs;
                    }
                    opened.presentation.slides[idx].shapes.push(shape);
                }
                SlideBlock::Table(tbl) => {
                    let sid = fresh_shape_id();
                    let slide_tbl = flow_table_to_slide_table(tbl, visual_frame.width);
                    opened.presentation.slides[idx].shapes.push(ibis_doc_model::Shape {
                        id: sid,
                        frame: visual_frame,
                        rotation: 0,
                        flip_h: false,
                        flip_v: false,
                        kind: ShapeKind::Table(Box::new(slide_tbl)),
                        floating: None,
                    });
                }
                SlideBlock::Picture(mid, size, look) => {
                    let sid = fresh_shape_id();
                    let frame = fit_size_in_rect(size, visual_frame);
                    if !look.alt_text.title.is_empty() || !look.alt_text.description.is_empty() {
                        opened.presentation.slides[idx]
                            .shape_alt_texts
                            .insert(sid, look.alt_text.clone());
                    }
                    opened.presentation.slides[idx].shapes.push(ibis_doc_model::Shape {
                        id: sid,
                        frame,
                        rotation: 0,
                        flip_h: false,
                        flip_v: false,
                        kind: ShapeKind::Picture(ibis_doc_model::Picture {
                            media: mid,
                            crop: ibis_doc_model::Crop::default(),
                            stroke: None,
                            corner: 0,
                            duotone: None,
                            clear: None,
                            luminance: None,
                            clip: None,
                            shadow: None,
                            reflection: None,
                            soft_edge: None,
                            alt_text: look.alt_text.clone(),
                            decorative: false,
                            name: format!("Picture {}", sid.0),
                            geometry: None,
                            effects: ibis_doc_model::BlipEffects::default(),
                            tile: None,
                            fill_rect: None,
                            artistic_effect: None,
                        }),
                        floating: None,
                    });
                }
                SlideBlock::Chart(src_chart) => {
                    let sid = fresh_shape_id();
                    let kind = match src_chart.groups.as_slice() {
                        [group] if matches!(group.kind, ChartType::Pie { .. } | ChartType::Doughnut { .. }) => NewChartKind::Pie,
                        [group] if matches!(group.kind, ChartType::Bar { horizontal: true, .. }) => NewChartKind::ClusteredBar,
                        [group] if matches!(group.kind, ChartType::Line) => NewChartKind::Line,
                        _ => NewChartKind::ClusteredColumn,
                    };
                    let mut chart_shape = opened.source.new_chart_kind(
                        &opened.presentation,
                        slide_id,
                        sid,
                        visual_frame,
                        kind,
                    )?;
                    if let ShapeKind::Chart(cbox) = &mut chart_shape.kind {
                        let mut chart = (**cbox).clone();
                        let is_mermaid = src_chart.mermaid_source.as_str().is_some();
                        let chart_data = if is_mermaid {
                            crate::save::pptx_chart_data::workbook_from_mermaid_chart(src_chart)
                        } else {
                            crate::save::pptx_chart_data::workbook_from_chart(src_chart)
                        };
                        if let Ok(chart_data) = chart_data {
                            if let Ok(wb_pres) = crate::read(&chart_data.workbook) {
                                chart = crate::save::pptx_chart_data::chart_from_data(
                                    &chart,
                                    &wb_pres,
                                    &chart_data.series,
                                );
                                chart.title = src_chart.title.clone();
                                copy_chart_group_topology(&mut chart, src_chart);
                                if is_mermaid && kind == NewChartKind::Pie {
                                    chart.legend = src_chart.legend.clone();
                                }
                                if is_mermaid || kind == NewChartKind::ClusteredBar {
                                    apply_mermaid_value_range(&mut chart, chart_value_range(src_chart));
                                }
                                opened.source.set_chart_data(sid, chart.clone(), chart_data);
                            }
                        }
                        chart.mermaid_source = src_chart.mermaid_source.clone();
                        *cbox = Box::new(chart);
                    }
                    if let ShapeKind::Chart(chart) = &chart_shape.kind
                        && let Some(source) = chart.mermaid_source.as_str()
                    {
                        opened.presentation.slides[idx].shape_alt_texts.insert(
                            sid,
                            ibis_doc_model::AltText {
                                title: "__mermaid__".to_string(),
                                description: source.to_string(),
                            },
                        );
                    }
                    opened.presentation.slides[idx].shapes.push(chart_shape);
                }
            }
            if let Some(placement) = placement {
                let shape = &mut opened.presentation.slides[idx].shapes[before];
                shape.frame = placement.frame;
                shape.rotation = placement.rotation;
                shape.flip_h = placement.flip_h;
                shape.flip_v = placement.flip_v;
            }
        }

        let _ = notes;
    }

    Ok(opened)
}

/// Export a PowerPoint `Presentation` to Markdown (`---`-separated slides).
pub fn export_presentation_markdown(p: &Presentation) -> String {
    let mut out = format!("---\nmarp: true\npptx_size: [{},{}]\n---\n\n", p.slide_size.width, p.slide_size.height);
    for (idx, slide) in p.slides.iter().enumerate() {
        if idx > 0 {
            out.push_str("\n---\n\n");
        }
        out.push_str(PPTX_SLIDE_MARKER);
        out.push_str("\n\n");
        let mut emitted_title = false;
        for shape in &slide.shapes {
            let position = out.len();
            write_markdown_shape_placement(&mut out, shape);
            match &shape.kind {
                ShapeKind::TextBox(body) | ShapeKind::Auto(ibis_doc_model::AutoShape { text: Some(body), .. }) => {
                    let mut paragraphs = body.paragraphs.as_slice();
                    if !emitted_title
                        && let Some((first, remaining)) = paragraphs.split_first()
                    {
                        let first_line: String = first.runs.iter().map(|run| run.text.as_str()).collect();
                        if !first_line.trim().is_empty() && !is_literal_mermaid_fence(&first_line) {
                            out.push_str("# ");
                            out.push_str(first_line.trim());
                            out.push_str("\n\n");
                            emitted_title = true;
                            paragraphs = remaining;
                        }
                    }
                    for para in paragraphs {
                        serialize_slide_paragraph(para, &mut out);
                    }
                }
                ShapeKind::Auto(auto) if !matches!(auto.geometry, ibis_doc_model::Geometry::Formula(_)) => {}
                ShapeKind::Table(tbl) => serialize_slide_table(tbl, &mut out),
                ShapeKind::Picture(pic) => {
                    let alt = slide
                        .shape_alt_texts
                        .get(&shape.id)
                        .unwrap_or(&pic.alt_text);
                    if alt.title == "__mermaid__" {
                        write_mermaid_fence(&alt.description, &mut out);
                    } else if let Some(media) = p.media(pic.media) {
                        use base64::Engine as _;
                        let mime = match media.format {
                            MediaFormat::Png => "image/png",
                            MediaFormat::Jpeg => "image/jpeg",
                            MediaFormat::Gif => "image/gif",
                            MediaFormat::Webp => "image/webp",
                            MediaFormat::Svg => "image/svg+xml",
                            MediaFormat::Emf => "image/x-emf",
                            MediaFormat::Wmf => "image/x-wmf",
                            _ => "application/octet-stream",
                        };
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&media.data);
                        out.push_str(&format!("![{}](data:{mime};base64,{b64})\n\n", alt.description));
                    } else {
                        out.truncate(position);
                    }
                }
                ShapeKind::Chart(chart) => {
                    let from_alt = slide
                        .shape_alt_texts
                        .get(&shape.id)
                        .filter(|alt| alt.title == "__mermaid__")
                        .map(|alt| alt.description.as_str());
                    let source = from_alt.or_else(|| chart.mermaid_source.as_str());
                    if let Some(source) = source.filter(|source| mermaid_source_matches(chart, source)) {
                        write_mermaid_fence(source, &mut out);
                    } else {
                        serialize_chart(chart, &mut out);
                    }
                }
                _ => out.truncate(position),
            }
        }
        if !slide.notes.trim().is_empty() {
            out.push_str("<!-- notes: ");
            out.push_str(slide.notes.trim());
            out.push_str(" -->\n\n");
        }
    }
    out
}

fn serialize_slide_paragraph(para: &ibis_doc_model::Paragraph, out: &mut String) {
    let text: String = para.runs.iter().map(|r| r.text.as_str()).collect();
    if is_literal_mermaid_fence(&text) {
        out.push_str(&text);
        out.push_str("\n\n");
        return;
    }
    if text.trim().is_empty() {
        return;
    }
    if let Some(b) = &para.bullet {
        if b.auto.is_some() {
            out.push_str("1. ");
        } else {
            out.push_str("- ");
        }
    }
    out.push_str(text.trim());
    out.push('\n');
    if para.bullet.is_none() {
        out.push('\n');
    }
}

fn split_markdown_slides(text: &str) -> Vec<String> {
    let mut body = text.trim();
    if let Some(after) = body.strip_prefix("---") {
        if after.starts_with('\n') || after.starts_with("\r\n") {
            let mut off = 3;
            for line in after.split_inclusive('\n') {
                off += line.len();
                if line.trim() == "---" {
                    body = body[off..].trim();
                    break;
                }
            }
        }
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut in_fence = None;
    for line in body.lines() {
        let t = line.trim();
        if !is_fenced_line(t, &mut in_fence) && (t == "---" || t == "***" || t == "___") {
            let joined = cur.join("\n").trim().to_string();
            if !joined.is_empty() {
                chunks.push(joined);
            }
            cur.clear();
        } else {
            cur.push(line);
        }
    }
    let last = cur.join("\n").trim().to_string();
    if !last.is_empty() {
        chunks.push(last);
    }

    if chunks.len() <= 1 && !body.starts_with(PPTX_SLIDE_MARKER) {
        // Fallback: split on top-level `# ` headings if multiple exist.
        let mut h_chunks: Vec<String> = Vec::new();
        let mut h_cur: Vec<&str> = Vec::new();
        let mut in_f = None;
        for line in body.lines() {
            let t = line.trim();
            if !is_fenced_line(t, &mut in_f) && line.starts_with("# ") && !h_cur.is_empty() {
                let s = h_cur.join("\n").trim().to_string();
                if !s.is_empty() {
                    h_chunks.push(s);
                }
                h_cur.clear();
            }
            h_cur.push(line);
        }
        let s = h_cur.join("\n").trim().to_string();
        if !s.is_empty() {
            h_chunks.push(s);
        }
        if h_chunks.len() > 1 {
            return h_chunks;
        }
    }

    if chunks.is_empty() {
        vec![String::new()]
    } else {
        chunks
    }
}

fn extract_slide_notes(chunk: &str) -> (String, Option<String>) {
    let mut body_lines: Vec<&str> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut fence = None;
    for line in chunk.lines() {
        let t = line.trim();
        if is_fenced_line(t, &mut fence) {
            body_lines.push(line);
            continue;
        }
        if let Some(inner) = t.strip_prefix("<!-- notes:").and_then(|s| s.strip_suffix("-->")) {
            let note = inner.trim();
            if !note.is_empty() {
                notes.push(note.to_string());
            }
        } else {
            body_lines.push(line);
        }
    }
    let notes_opt = if notes.is_empty() {
        None
    } else {
        Some(notes.join("\n"))
    };
    (body_lines.join("\n"), notes_opt)
}

fn shape_text_body_mut(kind: &mut ShapeKind) -> Option<&mut ibis_doc_model::TextBody> {
    match kind {
        ShapeKind::TextBox(body) => Some(body),
        ShapeKind::Auto(auto) => {
            if auto.text.is_none() {
                auto.text = Some(ibis_doc_model::TextBody::default());
            }
            auto.text.as_mut()
        }
        _ => None,
    }
}

pub(crate) fn flow_paragraph_plain_text(p: &FlowParagraph) -> String {
    let mut out = String::new();
    for r in &p.runs {
        if let FlowRunKind::Text(t) = &r.kind {
            out.push_str(t);
        }
    }
    out
}

fn flow_paragraph_to_slide_paragraph(p: &FlowParagraph) -> Option<ibis_doc_model::Paragraph> {
    let mut code = String::new();
    for run in &p.runs {
        match &run.kind {
            FlowRunKind::Text(text) => code.push_str(text),
            FlowRunKind::LineBreak => code.push('\n'),
            FlowRunKind::Tab => code.push('\t'),
            _ => {}
        }
    }
    if is_literal_mermaid_fence(&code) {
        return Some(ibis_doc_model::Paragraph {
            runs: vec![ibis_doc_model::Run { text: code, style: Default::default() }],
            ..Default::default()
        });
    }

    let mut runs: Vec<ibis_doc_model::Run> = Vec::new();
    for r in &p.runs {
        if let FlowRunKind::Text(t) = &r.kind {
            runs.push(ibis_doc_model::Run {
                text: t.clone(),
                style: ibis_doc_model::RunStyle {
                    bold: r.format.bold,
                    italic: r.format.italic,
                    font_size: Some(1800),
                    ..ibis_doc_model::RunStyle::default()
                },
            });
        }
    }
    if runs.is_empty() {
        return None;
    }
    let bullet = p.list.map(|ListRef { num_id, .. }| ibis_doc_model::Bullet {
        text: if num_id == 2 { "1.".to_string() } else { "•".to_string() },
        picture: None,
        size: 100_000,
        color: None,
        auto: None,
        symbol: None,
        font: None,
    });
    Some(ibis_doc_model::Paragraph {
        runs,
        bullet,
        space_after: 600,
        ..ibis_doc_model::Paragraph::default()
    })
}

fn serialize_slide_table(table: &ibis_doc_model::Table, out: &mut String) {
    if table.rows.is_empty() || table.columns.is_empty() {
        return;
    }
    let columns = table.columns.len();
    let mut vertical = HashMap::new();
    for (r, row) in table.rows.iter().enumerate() {
        out.push('|');
        for c in 0..columns {
            out.push(' ');
            if (r, c) == (0, 0) && !table.style_options.first_row {
                out.push_str(PPTX_NO_TABLE_HEADER);
                out.push(' ');
            }
            if let Some(cell) = row.cells.get(c).filter(|cell| !cell.merged) {
                let text = cell.text.paragraphs.iter()
                    .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect::<String>())
                    .collect::<Vec<_>>().join(" ");
                out.push_str(&text.trim().replace('|', "\\|").replace('\n', "<br>"));
                let span = (cell.column_span as usize).max(1).min(columns - c);
                let row_span = (cell.row_span as usize).max(1).min(table.rows.len() - r);
                if span > 1 {
                    out.push_str(&format!(" <!-- colspan:{span} -->"));
                }
                if row_span > 1 {
                    vertical.insert(c, (r + row_span, span));
                    out.push_str(" <!-- vmerge:start -->");
                }
            } else if let Some(&(end, span)) = vertical.get(&c).filter(|&&(end, _)| r < end) {
                if span > 1 {
                    out.push_str(&format!("<!-- colspan:{span} --> "));
                }
                out.push_str("<!-- vmerge:continue -->");
                if r + 1 == end {
                    vertical.remove(&c);
                }
            }
            out.push_str(" |");
        }
        out.push('\n');
        if r == 0 {
            out.push('|');
            for _ in 0..columns {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.push('\n');
}

fn flow_table_to_slide_table(tbl: &FlowTable, total_width: Emu) -> ibis_doc_model::Table {
    let cols = tbl.columns.len().max(1);
    let columns = vec![total_width / cols as i64; cols];
    let covered = ibis_doc_model::TableCell {
        text: ibis_doc_model::TextBody::default(),
        fill: Fill::None,
        borders: ibis_doc_model::CellBorders::default(),
        column_span: 1,
        row_span: 1,
        merged: true,
    };
    let mut vertical: HashMap<usize, usize> = HashMap::new();
    let mut rows: Vec<ibis_doc_model::TableRow> = Vec::new();
    for row in &tbl.rows {
        let mut cells = Vec::new();
        let mut continuing = HashMap::new();
        for cell in &row.cells {
            let column = cells.len();
            let span = cell.span.max(1);
            if cell.merge == VerticalMerge::Continue && let Some(&origin) = vertical.get(&column) {
                rows[origin].cells[column].row_span += 1;
                continuing.insert(column, origin);
                cells.extend(std::iter::repeat_n(covered.clone(), span as usize));
                continue;
            }
            if cell.merge == VerticalMerge::Start {
                continuing.insert(column, rows.len());
            }
            cells.push(ibis_doc_model::TableCell {
                text: ibis_doc_model::TextBody {
                    paragraphs: cell.blocks.iter().filter_map(|block| match block {
                        Block::Paragraph(paragraph) => flow_paragraph_to_slide_paragraph(paragraph),
                        _ => None,
                    }).collect(),
                    ..ibis_doc_model::TextBody::default()
                },
                fill: Fill::None,
                borders: ibis_doc_model::CellBorders::default(),
                column_span: span,
                row_span: 1,
                merged: false,
            });
            cells.extend(std::iter::repeat_n(covered.clone(), (span - 1) as usize));
        }
        rows.push(ibis_doc_model::TableRow { height: 370_840, cells, source: None });
        vertical = continuing;
    }
    ibis_doc_model::Table {
        columns,
        rows,
        column_sources: Vec::new(),
        style_options: ibis_doc_model::TableStyleOptions {
            first_row: tbl.rows.first().is_some_and(|row| row.header),
            ..Default::default()
        },
        style_id: None,
    }
}

fn fit_size_in_rect(size: EmuSize, rect: EmuRect) -> EmuRect {
    let w = size.width.max(1);
    let h = size.height.max(1);
    let rw = rect.width.max(1);
    let rh = rect.height.max(1);
    let scale = (rw as f64 / w as f64).min(rh as f64 / h as f64).min(1.0);
    let fw = (w as f64 * scale) as i64;
    let fh = (h as f64 * scale) as i64;
    EmuRect::new(
        rect.x + (rw - fw) / 2,
        rect.y + (rh - fh) / 2,
        fw,
        fh,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_layout_markdown_comments_do_not_become_document_text() {
        let markdown = b"<!-- fixed-layout-page: 1 -->\n\nShared document\n\n<!-- fixed-layout-diagnostic: Page 1: no extractable text -->\n";
        let opened = crate::open(import_markdown_to_docx_bytes(markdown).unwrap()).unwrap();
        let flow = opened.presentation.flow.unwrap();
        let text = ibis_doc_model::flow_command::all_paragraphs(&flow).iter().map(|p| p.text()).collect::<Vec<_>>();
        assert_eq!(text, ["Shared document"]);
    }

    #[test]
    fn docx_markdown_exports_an_anchored_picture() {
        let emf = include_bytes!("../../ibis-emf/tests/fixtures/shapes.emf");
        let image = format!("![floating](data:image/x-emf;base64,{}){{width=40mm height=30mm}}", encode_base64(emf));
        let original = import_markdown_to_docx_bytes(image.as_bytes()).unwrap();
        let mut package = ibis_ooxml::Package::open(original).unwrap();
        let part = "word/document.xml";
        let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
        let anchored = xml
            .replace("<wp:inline", "<wp:anchor")
            .replace("</wp:inline>", "</wp:anchor>")
            .replace("<wp:docPr", "<wp:wrapTopAndBottom/><wp:docPr");
        assert!(anchored.contains("<wp:anchor"));
        assert!(anchored.contains("<wp:wrapTopAndBottom/>"));
        package.set_bytes(part, anchored.into_bytes());
        let docx = package.save(ibis_ooxml::SaveOptions::default()).unwrap();
        assert_eq!(export_docx_to_markdown(&docx).unwrap().trim(), image);
    }

    #[test]
    fn markdown_picture_mime_matches_tiff_and_webp() {
        for (format, mime) in [(MediaFormat::Tiff, "tiff"), (MediaFormat::Webp, "webp")] {
            let data = b"unchanged bytes";
            let mut presentation = Presentation::default();
            let id = presentation.add_media(format, data.to_vec());
            let markdown = serialize_picture(&presentation.media, id, EmuSize::new(36000, 36000), &Default::default());
            let uri = markdown.strip_prefix("![](").unwrap().split_once(')').unwrap().0;
            assert!(uri.starts_with(&format!("data:image/{mime};base64,")));
            assert_eq!(decode_data_uri_image(uri), Some((format, data.to_vec())));
        }
    }

    #[test]
    fn office_markdown_round_trip_preserves_metafile_bytes() {
        for (format, mime, original) in [
            (MediaFormat::Emf, "x-emf", include_bytes!("../../ibis-emf/tests/fixtures/shapes.emf").as_slice()),
            (MediaFormat::Wmf, "x-wmf", include_bytes!("../../ibis-emf/tests/fixtures/shapes.wmf").as_slice()),
        ] {
            let snippet = format!(
                "![drawing](data:image/{mime};base64,{}){{width=40mm height=30mm}}\n",
                encode_base64(original)
            );
            for compile in [import_markdown_to_docx_bytes, import_markdown_to_pptx_bytes] {
                let package = compile(snippet.as_bytes()).unwrap();
                let opened = crate::open(package).unwrap();
                let markdown = export_presentation_to_markdown(&opened.presentation).unwrap();
                assert!(markdown.contains(&format!("data:image/{mime};base64,")));
                let rebuilt = crate::open(compile(markdown.as_bytes()).unwrap()).unwrap();
                assert!(rebuilt.presentation.media.iter().any(|media| {
                    media.format == format && media.data.as_ref() == original
                }));
            }
        }
    }

    #[test]
    fn nested_table_round_trips_with_surrounding_text_and_combined_cells() {
        let input = br"| Category | Details |
| --- | --- |
| Domestic | Before<br><!-- nested-table: \| Group &amp; type &lt; 2 &lt;!-- colspan:2 --&gt; \| \|<br>\| --- \| --- \|<br>\| X\\\|Y \| 12&lt;br&gt;8 \| --><br>After |
";
        let docx = import_markdown_to_docx_bytes(input).unwrap();
        let exported = export_docx_to_markdown(&docx).unwrap();
        let rebuilt = import_markdown_to_docx_bytes(exported.as_bytes()).unwrap();
        assert_eq!(export_docx_to_markdown(&rebuilt).unwrap(), exported);

        let presentation = crate::read(&rebuilt).unwrap();
        let Block::Table(outer) = &presentation.flow.as_ref().unwrap().body[0] else {
            panic!("expected an outer table");
        };
        let [Block::Paragraph(before), Block::Table(inner), Block::Paragraph(after)] =
            outer.rows[1].cells[1].blocks.as_slice()
        else {
            panic!("expected a nested table between two paragraphs");
        };
        assert_eq!(before.text(), "Before");
        assert_eq!(after.text(), "After");
        assert_eq!(inner.rows[0].cells[0].span, 2);
        assert!(matches!(&inner.rows[0].cells[0].blocks[0], Block::Paragraph(p) if p.text() == "Group & type < 2"));
        assert!(matches!(&inner.rows[1].cells[0].blocks[0], Block::Paragraph(p) if p.text() == "X|Y"));
        assert!(matches!(&inner.rows[1].cells[1].blocks[0], Block::Paragraph(p)
            if p.runs.iter().any(|r| matches!(r.kind, FlowRunKind::LineBreak))));
    }

    #[test]
    fn vertical_table_merges_round_trip_and_keep_occupied_cells() {
        let markdown = b"| Group | Type | Count |
| --- | --- | --- |
| Domestic <!-- colspan:2 --> <!-- vmerge:start --> | | 12 |
| <!-- colspan:2 --> <!-- vmerge:continue --> | | 8 |
| <!-- vmerge:continue --> | Width mismatch | 3 |
| Keep <!-- vmerge:start --> | Other | 4 |
| Occupied <!-- vmerge:continue --> | Other | 5 |
| <!-- vmerge:continue --> | Orphan | 6 |
";
        let docx = import_markdown_to_docx_bytes(markdown).unwrap();
        let exported = export_docx_to_markdown(&docx).unwrap();
        assert!(exported.contains("<!-- colspan:2 --> <!-- vmerge:continue -->"));
        assert!(exported.contains("| Occupied |"));
        let rebuilt = import_markdown_to_docx_bytes(exported.as_bytes()).unwrap();
        assert_eq!(export_docx_to_markdown(&rebuilt).unwrap(), exported);

        let presentation = crate::read(&rebuilt).unwrap();
        let Block::Table(table) = &presentation.flow.as_ref().unwrap().body[0] else {
            panic!("expected a table");
        };
        let merges: Vec<_> = table.rows.iter().map(|row| {
            let cell = &row.cells[0];
            (cell.span, cell.merge)
        }).collect();
        assert_eq!(merges, [
            (1, VerticalMerge::None),
            (2, VerticalMerge::Start),
            (2, VerticalMerge::Continue),
            (1, VerticalMerge::None),
            (1, VerticalMerge::Start),
            (1, VerticalMerge::None),
            (1, VerticalMerge::None),
        ]);
    }

    #[test]
    fn horizontal_table_merge_round_trips_through_docx() {
        let input = b"| Group <!-- colspan:2 --> | | Total |
| :---: | --- | ---: |
| A | B | 2 |
| Keep <!-- colspan:2 --> | Occupied | 3 |
";
        let original = import_markdown_to_docx_bytes(input).unwrap();
        let markdown = export_docx_to_markdown(&original).unwrap();
        assert!(markdown.contains("<!-- colspan:2 --> |  |"));
        let rebuilt = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        let presentation = crate::read(&rebuilt).unwrap();
        let Block::Table(table) = &presentation.flow.as_ref().unwrap().body[0] else {
            panic!("expected a table");
        };
        assert_eq!(table.columns.len(), 3);
        let spans = |row: usize| table.rows[row].cells.iter().map(|cell| cell.span).collect::<Vec<_>>();
        assert_eq!(spans(0), [2, 1]);
        assert_eq!(spans(1), [1, 1, 1]);
        assert_eq!(spans(2), [1, 1, 1]);
        assert!(matches!(&table.rows[2].cells[1].blocks[0], Block::Paragraph(p) if p.text() == "Occupied"));

        let slide = flow_table_to_slide_table(table, 6 * EMU_PER_INCH);
        assert_eq!(slide.rows[0].cells.len(), 3);
        assert_eq!(slide.rows[0].cells[0].column_span, 2);
        assert!(slide.rows[0].cells[1].merged);
        assert!(!slide.rows[0].cells[2].merged);
    }

    #[test]
    fn bibliography_marker_round_trips_with_surrounding_paragraphs() {
        let input = "Before.\n[[bibliography:1041]]\nAfter.\n\n[[bibliography]]\n";
        let docx = import_markdown_to_docx_bytes(input.as_bytes()).unwrap();
        let markdown = export_docx_to_markdown(&docx).unwrap();
        assert_eq!(markdown, "Before.\n\n[[bibliography:1041]]\n\nAfter.\n\n[[bibliography]]\n");
        let rebuilt = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        let document = crate::read_docx(&rebuilt).unwrap();
        let fields = document.flow.as_ref().unwrap().body.iter().filter_map(|block| {
            match block {
                Block::Paragraph(p) if p.field_end => p.field_start.as_deref(),
                _ => None,
            }
        }).collect::<Vec<_>>();
        assert_eq!(fields, [r"BIBLIOGRAPHY \l 1041", "BIBLIOGRAPHY"]);

        let mut package = ibis_ooxml::Package::open(docx).unwrap();
        let part = "word/document.xml";
        let mut xml = String::from_utf8(package.read(part).unwrap()).unwrap();
        let field = xml.find("BIBLIOGRAPHY").unwrap();
        let end = field + xml[field..].find("</w:p>").unwrap();
        xml.insert_str(end, "<w:r><w:t>Inline suffix.</w:t></w:r>");
        package.set_bytes(part, xml.into_bytes());
        let mixed = package.save(ibis_ooxml::SaveOptions::default()).unwrap();
        let exported = export_docx_to_markdown(&mixed).unwrap();
        assert!(!exported.contains("[[bibliography:1041]]"));
        assert!(exported.contains("Inline suffix."));
        assert!(exported.contains("[[bibliography]]"));
    }

    #[test]
    fn word_bibliography_sources_round_trip_through_document_metadata() {
        let xml = r#"<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography">
<b:Source><b:Tag>Tanaka_2025</b:Tag><b:SourceType>Book</b:SourceType>
<b:Author><b:Author><b:NameList><b:Person><b:Last>田中</b:Last></b:Person></b:NameList></b:Author></b:Author>
<b:Title>研究と実践 &amp; 検証</b:Title><b:Year>2025</b:Year></b:Source>
</b:Sources>"#;
        let markdown = format!("```word-bibliography\n{xml}\n```\n\n参照 {{{{cite:Tanaka_2025:1041}}}}。\n");
        let bytes = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        let package = ibis_ooxml::Package::open(bytes.clone()).unwrap();
        let main = "word/document.xml";
        let source_part = package.relationships(main).unwrap().into_iter()
            .filter(|rel| crate::bibliography::is_custom_xml(&rel.kind))
            .filter_map(|rel| rel.target_part(main))
            .find(|part| package.read(part).unwrap() == xml.as_bytes())
            .expect("Word bibliography part linked to document");
        let props = package.relationships(&source_part).unwrap().into_iter()
            .find(|rel| rel.kind.ends_with("/customXmlProps")).unwrap()
            .target_part(&source_part).unwrap();
        assert!(String::from_utf8(package.read(&props).unwrap()).unwrap().contains("ds:schemaRef"));

        let model = crate::read_docx(&bytes).unwrap();
        let flow = model.flow.as_ref().unwrap();
        assert_eq!(flow.bibliography_sources, [xml]);
        assert_eq!(flow.body.len(), 1);
        let exported = export_docx_to_markdown(&bytes).unwrap();
        assert_eq!(exported, markdown);
        let rebuilt = import_markdown_to_docx_bytes(exported.as_bytes()).unwrap();
        assert_eq!(export_docx_to_markdown(&rebuilt).unwrap(), markdown);
        assert!(bibliography_fence("```word-bibliography\n<Sources/>\n```").is_none());
    }

    #[test]
    fn citation_tags_locales_and_pages_round_trip_as_word_fields() {
        let input = "See {{cite:Smith-2024}}, {{cite:Smith-2024#12}}, and {{cite:Tanaka_2025:1041#12–15}}.";
        let docx = import_markdown_to_docx_bytes(input.as_bytes()).unwrap();
        let presentation = crate::read(&docx).unwrap();
        let Block::Paragraph(paragraph) = &presentation.flow.as_ref().unwrap().body[0] else {
            panic!("expected a paragraph");
        };
        let fields = paragraph.runs.iter().filter_map(|run| match &run.kind {
            FlowRunKind::Field { instruction, .. } => Some(instruction.as_str()),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(fields, [
            "CITATION Smith-2024",
            r#"CITATION Smith-2024 \p "12""#,
            r#"CITATION Tanaka_2025 \l 1041 \p "12–15""#,
        ]);
        assert_eq!(export_docx_to_markdown(&docx).unwrap().trim(), input);
        assert_eq!(format_markdown_inline_field(r#"CITATION "Tanaka_2025" \l 1041"#, "(田中, 2025)"),
                   "{{cite:Tanaka_2025:1041}}");
        assert_eq!(format_markdown_inline_field(r"CITATION Tanaka_2025 \p 12-15 \l 1041", ""),
                   "{{cite:Tanaka_2025:1041#12-15}}");
        assert_eq!(format_markdown_inline_field(r"CITATION Tanaka_2025 \p 12 \m Smith-2024 \f see", "(田中, 2025)"),
                   "(田中, 2025)");
        assert!(markdown_inline_field("{{cite:Tanaka_2025#12-}}").is_none());
        assert!(markdown_inline_field("{{cite:Tanaka_2025:invalid}}").is_none());
    }

    #[test]
    fn multisource_citations_keep_order_and_per_source_switches() {
        let input = "See {{cite:Tanaka_2025:1041#12–15;Smith-2024:1033#24;Suzuki_2023}}.";
        let docx = import_markdown_to_docx_bytes(input.as_bytes()).unwrap();
        let presentation = crate::read(&docx).unwrap();
        let Block::Paragraph(paragraph) = &presentation.flow.as_ref().unwrap().body[0] else {
            panic!("expected a paragraph");
        };
        assert!(paragraph.runs.iter().any(|run| matches!(&run.kind,
            FlowRunKind::Field { instruction, .. }
                if instruction == r#"CITATION Tanaka_2025 \l 1041 \p "12–15" \m Smith-2024 \l 1033 \p "24" \m Suzuki_2023"#
        )));
        assert_eq!(export_docx_to_markdown(&docx).unwrap().trim(), input);
        assert!(markdown_inline_field("{{cite:Tanaka_2025;}}").is_none());
        assert!(format_markdown_citation("Tanaka_2025", &[r"\m", "Smith-2024", r"\p"]).is_none());
    }

    #[test]
    fn caption_numbers_and_forward_references_round_trip_and_renumber() {
        fn fields(data: &[u8]) -> Vec<(String, String)> {
            let presentation = crate::read(data).unwrap();
            presentation.flow.as_ref().unwrap().body.iter()
                .filter_map(|block| match block {
                    Block::Paragraph(paragraph) => Some(&paragraph.runs),
                    _ => None,
                })
                .flatten()
                .filter_map(|run| match &run.kind {
                    FlowRunKind::Field { instruction, shown } => Some((instruction.clone(), shown.clone())),
                    _ => None,
                })
                .collect()
        }

        let input = b"See equation ({{ref:eq_energy_number}}).

$$E=mc^2$$ {#eq_energy}

{{seq:equation}} {#eq_energy_number}

Figure {{seq:figure}} {#fig_setup}

Table {{seq:table}} {#tbl_results}
";
        let original = import_markdown_to_docx_bytes(input).unwrap();
        let expected = [
            (r"REF eq_energy_number \h".into(), "1".into()),
            ("SEQ Equation".into(), "1".into()),
            ("SEQ Figure".into(), "1".into()),
            ("SEQ Table".into(), "1".into()),
        ];
        assert_eq!(fields(&original), expected);
        let markdown = export_docx_to_markdown(&original).unwrap();
        assert!(markdown.contains("{{seq:equation}} {#eq_energy_number}"));
        assert!(markdown.contains("{{ref:eq_energy_number}}"));
        let rebuilt = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        assert_eq!(fields(&rebuilt), expected);

        let earlier = format!("{{{{seq:equation}}}} {{#eq_earlier_number}}\n\n{markdown}");
        let renumbered = import_markdown_to_docx_bytes(earlier.as_bytes()).unwrap();
        assert_eq!(
            fields(&renumbered),
            [
                ("SEQ Equation".into(), "1".into()),
                (r"REF eq_energy_number \h".into(), "2".into()),
                ("SEQ Equation".into(), "2".into()),
                ("SEQ Figure".into(), "1".into()),
                ("SEQ Table".into(), "1".into()),
            ],
        );
    }

    #[test]
    fn equation_bookmarks_and_internal_links_survive_docx_round_trip() {
        let input = br"See the [energy equation](#eq_energy).

$$
E=mc^2
$$ {#eq_energy}

$$a=b$$ {#eq_inline_source}

Following paragraph.
";
        let docx = import_markdown_to_docx_bytes(input).unwrap();
        // Word can move a paragraph-wide bookmark start out of the paragraph.
        let mut package = ibis_ooxml::Package::open(docx).unwrap();
        let mut xml = String::from_utf8(package.read("word/document.xml").unwrap()).unwrap();
        let name = xml.find("w:name=\"eq_energy\"").unwrap();
        let start = xml[..name].rfind("<w:bookmarkStart ").unwrap();
        let end = name + xml[name..].find("/>").unwrap() + 2;
        let paragraph = xml[..start].rfind("<w:p ").or_else(|| xml[..start].rfind("<w:p>")).unwrap();
        let bookmark = xml[start..end].to_owned();
        xml.replace_range(start..end, "");
        xml.insert_str(paragraph, &bookmark);
        package.set_bytes("word/document.xml", xml.into_bytes());
        let docx = package.save(ibis_ooxml::SaveOptions::default()).unwrap();
        let markdown = export_docx_to_markdown(&docx).unwrap();
        assert!(markdown.contains("[energy equation](#eq_energy)"));
        assert!(markdown.contains("$$ {#eq_energy}"));
        assert!(markdown.contains("$$ {#eq_inline_source}"));
        assert!(markdown.contains("Following paragraph."));

        let mut opened = crate::open(docx).unwrap();
        let flow = std::sync::Arc::make_mut(opened.presentation.flow.as_mut().unwrap());
        let Block::Paragraph(equation) = &mut flow.body[1] else { panic!("expected an equation") };
        equation.bookmarks.clear();
        let unbookmarked = opened.source.save(&opened.presentation, SaveOptions::default()).unwrap();
        let package = ibis_ooxml::Package::open(unbookmarked).unwrap();
        let xml = String::from_utf8(package.read("word/document.xml").unwrap()).unwrap();
        assert!(!xml.contains("w:name=\"eq_energy\""));
        assert!(xml.contains("w:name=\"eq_inline_source\""));

        let rebuilt = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        let presentation = crate::read(&rebuilt).unwrap();
        let flow = presentation.flow.unwrap();
        let [Block::Paragraph(reference), Block::Paragraph(equation), ..] = flow.body.as_slice() else {
            panic!("expected a reference followed by a display equation");
        };
        assert!(reference.runs.iter().any(|run| {
            matches!(&run.link, Some(Link::Anchor(anchor)) if anchor == "eq_energy")
        }));
        assert_eq!(equation.bookmarks, ["eq_energy"]);
        assert!(matches!(equation.runs[0].kind, FlowRunKind::Math { display: true, .. }));
    }

    #[test]
    fn docx_markdown_round_trip_keeps_matrix_and_equation_array_structure() {
        // Inject independent OMML instead of using our Markdown or OMML writer
        // to construct the input equations.
        let equations = r#"
<w:p><m:oMathPara><m:oMath><m:m>
  <m:mr><m:e><m:r><m:t>a</m:t></m:r></m:e><m:e><m:r><m:t>b</m:t></m:r></m:e></m:mr>
  <m:mr><m:e><m:r><m:t>c</m:t></m:r></m:e><m:e><m:r><m:t>d</m:t></m:r></m:e></m:mr>
</m:m></m:oMath></m:oMathPara></w:p>
<w:p><m:oMathPara><m:oMath><m:eqArr>
  <m:e><m:r><m:t>x</m:t></m:r><m:r><m:rPr><m:aln/></m:rPr><m:t>=1</m:t></m:r></m:e>
  <m:e><m:r><m:rPr><m:aln m:val="0"/></m:rPr><m:t>yy</m:t></m:r><m:box>
    <m:boxPr><m:opEmu/><m:aln/></m:boxPr><m:e><m:r><m:t>=</m:t></m:r></m:e>
  </m:box><m:r><m:t>2</m:t></m:r></m:e>
</m:eqArr></m:oMath></m:oMathPara></w:p>"#;
        let template = import_markdown_to_docx_bytes(b"Math\n").unwrap();
        let mut package = ibis_ooxml::Package::open(template).unwrap();
        let xml = String::from_utf8(package.read("word/document.xml").unwrap()).unwrap();
        let xml = xml.replace(
            "<w:body>",
            &format!(
                "<w:body xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">{equations}"
            ),
        );
        package.set_bytes("word/document.xml", xml.into_bytes());
        let original = package.save(ibis_ooxml::SaveOptions::default()).unwrap();

        let markdown = export_docx_to_markdown(&original).unwrap();
        assert!(markdown.contains(r"\begin{matrix}a & b \\ c & d\end{matrix}"));
        assert!(markdown.contains(r"\begin{aligned}x&=1 \\ yy&=2\end{aligned}"));

        let rebuilt = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        let presentation = crate::read(&rebuilt).unwrap();
        let flow = presentation.flow.unwrap();
        let formulas: Vec<_> = flow.body.iter().filter_map(|block| {
            let Block::Paragraph(paragraph) = block else { return None };
            let [run] = paragraph.runs.as_slice() else { return None };
            let FlowRunKind::Math { math, display: true } = &run.kind else { return None };
            Some(math)
        }).collect();
        use ibis_doc_model::math::Math;
        let [Math::Matrix(matrix), Math::EqArray(rows)] = formulas.as_slice() else {
            panic!("expected an editable matrix and equation array: {formulas:?}");
        };
        let cells: Vec<Vec<_>> = matrix.iter().map(|row| row.iter().map(Math::linear).collect()).collect();
        assert_eq!(cells, [vec!["a", "b"], vec!["c", "d"]]);
        assert_eq!(rows.iter().map(Math::linear).collect::<Vec<_>>(), ["x&=1", "yy&=2"]);
    }

    #[test]
    fn docx_markdown_round_trip_keeps_explicit_operator_limits() {
        fn limit_placement(data: &[u8]) -> Vec<(bool, bool, String, String)> {
            let presentation = crate::read(data).unwrap();
            let mut placements = Vec::new();
            for block in &presentation.flow.as_ref().unwrap().body {
                let Block::Paragraph(paragraph) = block else { continue };
                for run in &paragraph.runs {
                    if let FlowRunKind::Math { math, display } = &run.kind {
                        let operator = match math {
                            ibis_doc_model::math::Math::Row(items) => &items[0],
                            operator => operator,
                        };
                        let ibis_doc_model::math::Math::Nary {
                            stacked, sub: Some(sub), sup: Some(sup), ..
                        } = operator else {
                            panic!("expected editable operator with two limits: {operator:?}");
                        };
                        placements.push((*display, *stacked, sub.to_latex(), sup.to_latex()));
                    }
                }
            }
            placements
        }

        let markdown = br"Inline $\int\limits_{0}^{1} x$.

$$
\sum\nolimits_{i=0}^{n} x_i
$$
";
        let docx = import_markdown_to_docx_bytes(markdown).unwrap();
        let expected = vec![
            (false, true, "0".into(), "1".into()),
            (true, false, "i=0".into(), "n".into()),
        ];
        assert_eq!(limit_placement(&docx), expected);
        let exported = export_docx_to_markdown(&docx).unwrap();
        assert!(exported.contains(r"\int\limits_{0}^{1}"));
        assert!(exported.contains(r"\sum\nolimits_{i=0}^{n}"));
        let saved_again = import_markdown_to_docx_bytes(exported.as_bytes()).unwrap();
        assert_eq!(limit_placement(&saved_again), expected);
    }

    #[test]
    fn detects_markdown_vs_csv() {
        assert!(is_markdown(b"# Heading 1\n\nSome paragraph."));
        assert!(is_markdown(b"---\ntitle: Test\n---\nHello"));
        assert!(is_markdown(b"| A | B |\n| --- | --- |\n| 1 | 2 |\n"));
        assert!(!is_markdown(b"name,age,city\nAlice,30,Tokyo\nBob,25,Osaka\n"));
        assert!(!is_markdown(b"PK\x03\x04\x14\x00"));
    }

    #[test]
    fn rich_markdown_roundtrips_and_saves_to_valid_docx() {
        let md = r#"---
title: "2026年度 技術戦略レポート"
subtitle: "Cloudflare Artifacts × ibisOffice"
author: "masanork"
keywords: "markdown, ooxml, gitops"
size: A4
columns: 2
column_separator: true
style_set: modern
paragraph_spacing: relaxed
header: "機密文書 - {filename}"
footer: "{page}"
watermark: "社外秘"
track_changes: true
---

# 第1章 はじめに {#intro align=center}

本書は **太字**、*斜体*、***太字斜体***、~~取消線~~、++下線++、==蛍光ペン==、上付き x^2^、下付き H~2~O、および `inline_code()` を備えたリッチ Markdown 文書です[^1]。
また日本語組版として {漢字|かんじ} や ｜青空文庫《あおぞらぶんこ》、傍点《《強調表現》》、縦中横 [26]{.tcy}、リンク [sorane](https://docs.sorane.dev) にも対応しています。

変更履歴として {++追加されたテキスト++} と {--削除されたテキスト--}、およびレビューコメント対象語句{>>masanork: ここを要確認<<}を含みます。

> [!IMPORTANT]
> Git リポジトリ上の Markdown を直接 ibisOffice のページレイアウトで編集・保存できます。

- 箇条書きアイテム 1
  - ネストされたアイテム 1.1
- [ ] 未完了タスク
- [x] 完了済みタスク

1. 番号付きリスト 1
2. 番号付きリスト 2

$$
\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}
$$

```rust
fn main() {
    println!("Hello, ibisOffice Markdown!");
}
```

| 項目 | 2025年度 | 2026年度 |
| :--- | :---: | ---: |
| 売上高 | 120 | 180 |
| 営業利益 | 30 | 55 |

```chart
type: column
title: 年度別業績推移
categories: [Q1, Q2, Q3, Q4]
series:
  - name: 2025年度, values: [25, 30, 28, 37]
  - name: 2026年度, values: [40, 45, 42, 53]
```

\newpage

## 第2章 まとめ

インライン数式 $E = mc^2$ のテストです。

[^1]: 脚注の本文です。**書式**も使えます。
"#;

        // 1. Open as rich Markdown
        let opened = open_markdown(md.as_bytes().to_vec()).expect("open_markdown succeeds");
        let flow = opened.presentation.flow.as_ref().expect("flow document present");

        assert_eq!(flow.properties.title.as_deref(), Some("2026年度 技術戦略レポート"));
        assert_eq!(flow.properties.creator.as_deref(), Some("masanork"));
        assert_eq!(flow.page.columns.count, 2);
        assert!(flow.page.columns.separator);
        assert_eq!(flow.watermark.as_ref().map(|w| w.text.as_str()), Some("社外秘"));
        assert!(flow.track_changes);
        assert_eq!(flow.footnotes.len(), 1);
        assert_eq!(flow.comments.len(), 1);

        // 2. Save to .docx and re-read via read_docx to verify 100% OOXML compatibility!
        let docx_bytes = opened
            .source
            .save(&opened.presentation, SaveOptions::default())
            .expect("saving Markdown-opened document to .docx succeeds");
        let reread = crate::read_docx(&docx_bytes).expect("re-reading saved .docx succeeds");
        let reread_flow = reread.flow.as_ref().expect("reread flow");

        assert_eq!(reread_flow.properties.title.as_deref(), Some("2026年度 技術戦略レポート"));
        assert_eq!(reread_flow.page.columns.count, 2);
        assert_eq!(reread_flow.footnotes.len(), 1);
        assert_eq!(reread_flow.comments.len(), 1);
        assert_eq!(
            reread_flow.watermark.as_ref().map(|w| w.text.as_str()),
            Some("社外秘")
        );

        // Verify chart survived .docx save & reread with custom title and 2 series!
        let mut found_chart = None;
        for b in &reread_flow.body {
            if let Block::Paragraph(p) = b {
                for r in &p.runs {
                    if let FlowRunKind::Shape(s) = &r.kind
                        && let ShapeKind::Chart(c) = &s.kind
                    {
                        found_chart = Some((**c).clone());
                    }
                }
            }
        }
        let chart = found_chart.expect("chart saved and reread from .docx");
        assert_eq!(chart.title.as_ref().map(|t| t.text.as_str()), Some("年度別業績推移"));
        let series: Vec<&Series> = chart.groups.iter().flat_map(|g| &g.series).collect();
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].name, "2025年度");
        assert_eq!(series[1].name, "2026年度");
        assert_eq!(series[0].categories, vec!["Q1", "Q2", "Q3", "Q4"]);

        // 3. Export back to Markdown and verify key constructs roundtrip
        let exported_md = export_markdown(flow, &opened.presentation.media);
        assert!(exported_md.contains("title: 2026年度 技術戦略レポート"));
        assert!(exported_md.contains("columns: 2"));
        assert!(exported_md.contains("# 第1章 はじめに {#intro align=center}"));
        assert!(exported_md.contains("{漢字|かんじ}"));
        assert!(exported_md.contains("《《強調表現》》"));
        assert!(exported_md.contains("[26]{.tcy}"));
        assert!(exported_md.contains("{++追加されたテキスト++}"));
        assert!(exported_md.contains("{--削除されたテキスト--}"));
        assert!(exported_md.contains("{>>masanork: ここを要確認<<}"));
        assert!(exported_md.contains("> [!IMPORTANT]"));
        assert!(exported_md.contains("- [ ] 未完了タスク"));
        assert!(exported_md.contains("- [x] 完了済みタスク"));
        assert!(exported_md.contains("```chart"));
        assert!(exported_md.contains("[^1]: 脚注の本文です。**書式**も使えます。"));
    }

    #[test]
    fn detects_markdown_edge_cases() {
        assert!(is_markdown(b"1. First item\n2. Second item\n"));
        assert!(is_markdown(b"Hello\n\n---\n\nWorld\n"));
        assert!(is_markdown(b"Use `cargo test` to verify.\n"));
        assert!(is_markdown(b"This is ~~deprecated~~ and ==important==.\n"));
        assert!(!is_markdown(b""));
        assert!(!is_markdown(b"   \n\t\n"));
        assert!(!is_markdown(b"col1,col2,col3\n1,2,3\n4,5,6\n"));
        assert!(!is_markdown(b"plain text without any markdown markers\njust two lines\n"));
        assert!(!is_markdown(b"hello\x00world"));
    }

    #[test]
    fn loads_local_relative_images_and_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("ibis_md_img_test_{}", std::process::id()));
        let img_dir = dir.join("images");
        std::fs::create_dir_all(&img_dir).unwrap();

        // Minimal PNG header + IHDR (400x200) so detect_image_format & infer_image_dimensions_px succeed.
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&400u32.to_be_bytes());
        png.extend_from_slice(&200u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend_from_slice(&[0; 4]);
        let img_path = img_dir.join("chart.png");
        std::fs::write(&img_path, &png).unwrap();

        let outside = dir.parent().unwrap().join("ibis_md_outside_test.png");
        std::fs::write(&outside, &png).unwrap();

        let md = format!(
            "# Image Test\n\n![sample](images/chart.png)\n\n![escape](../ibis_md_outside_test.png)\n\n![abs]({})\n",
            outside.display()
        );
        let opened = open_markdown_with_base_dir(md.into_bytes(), Some(&dir)).expect("open_markdown_with_base_dir");
        let flow = opened.presentation.flow.as_ref().unwrap();

        let mut pictures = Vec::new();
        for b in &flow.body {
            if let Block::Paragraph(p) = b {
                for r in &p.runs {
                    if let FlowRunKind::Picture { media, size, look } = &r.kind {
                        pictures.push((*media, *size, look.alt_text.description.clone()));
                    }
                }
            }
        }
        assert_eq!(pictures.len(), 1, "only relative image inside base_dir should be loaded");
        assert_eq!(pictures[0].2, "sample");
        // 400x200 => 2:1 aspect ratio at 96 DPI (9525 EMU/px)
        assert_eq!(pictures[0].1.width, 400 * 9_525);
        assert_eq!(pictures[0].1.height, 200 * 9_525);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn fragment_blocks_roundtrip_for_clipboard() {
        let opened = crate::save::open(crate::blank::blank_file(crate::Kind::Document).unwrap()).unwrap();
        let mut doc = (**opened.presentation.flow.as_ref().unwrap()).clone();
        let mut media = opened.presentation.media.clone();

        let fragment = "## Subheading\n\nText with **bold**, ~~strike~~, and `code`.\n\n- item A\n- item B\n\n| K | V |\n| :--- | ---: |\n| x | 42 |\n\n```mermaid\npie\n  \"A\" : 70\n  \"B\" : 30\n```\n";
        let blocks = parse_markdown_fragment_blocks(fragment, &mut doc, &mut media).expect("parse fragment");
        assert!(blocks.len() >= 5);
        assert!(media.iter().any(|m| m.format == MediaFormat::Svg), "pie fragment should fall back to SVG");

        let exported = export_blocks_to_markdown(&blocks, &doc, &media);
        assert!(exported.contains("## Subheading"));
        assert!(exported.contains("**bold**"));
        assert!(exported.contains("~~strike~~"));
        assert!(exported.contains("`code`"));
        assert!(exported.contains("- item A"));
        assert!(exported.contains("| x | 42 |"));
        assert!(exported.contains("```mermaid\npie"));
    }

    #[test]
    fn parses_and_exports_alerts_nested_lists_and_tables() {
        let md = "\
> [!NOTE]
> Note body

> [!WARNING]
> Warning body

- level 0
  - level 1
    - level 2

| Left | Center | Right |
| :--- | :---: | ---: |
| a | b | c |
";
        let pres = read_markdown(md.as_bytes()).expect("read_markdown");
        let flow = pres.flow.as_ref().unwrap();
        let out = export_markdown(flow, &pres.media);
        assert!(out.contains("> [!NOTE]"));
        assert!(out.contains("> [!WARNING]"));
        assert!(out.contains("- level 0"));
        assert!(out.contains("  - level 1"));
        assert!(out.contains("    - level 2"));
        assert!(out.contains("| **Left** | **Center** | **Right** |"));
        assert!(out.contains("| :--- | :---: | ---: |"));
        assert!(out.contains("| a | b | c |"));
    }

    #[test]
    fn mermaid_fences_preserve_nested_delimiters_and_slide_markers() {
        let source = "zenuml\n```\n---\n# data\n<!-- notes: keep in source -->\n~~~\n";
        let markdown = format!("~~~~mermaid\n{source}~~~~\n");
        let expected = format!("````mermaid\n{source}````");
        let document = open_markdown(markdown.as_bytes().to_vec()).unwrap();
        let docx = document.source.save(&document.presentation, Default::default()).unwrap();
        let reopened = crate::read(&docx).unwrap();
        assert!(export_markdown(reopened.flow.as_ref().unwrap(), &reopened.media).contains(&expected));
        let slides = open_markdown_presentation(markdown.as_bytes().to_vec()).unwrap();
        assert_eq!(slides.presentation.slides.len(), 1);
        assert!(slides.presentation.slides[0].notes.is_empty());
        let pptx = slides.source.save(&slides.presentation, Default::default()).unwrap();
        let reopened = crate::read(&pptx).unwrap();
        assert!(export_presentation_markdown(&reopened).contains(&expected));
    }

    #[test]
    fn unsupported_and_empty_mermaid_fences_stay_code_blocks() {
        let md = "\
```mermaid
requirementDiagram
  requirement req1
```

```mermaid
```

```mermaid
C4Context
```
";
        let opened = open_markdown(md.as_bytes().to_vec()).expect("open_markdown");
        let flow = opened.presentation.flow.as_ref().expect("flow");
        let has_chart = flow.body.iter().any(|block| {
            matches!(
                block,
                Block::Paragraph(p)
                    if p.runs.iter().any(|r| matches!(&r.kind, FlowRunKind::Shape(s) if matches!(s.kind, ShapeKind::Chart(_))))
            )
        });
        assert!(!has_chart, "an unsupported fence must not become a chart");
        assert!(opened.presentation.media.iter().all(|m| m.format != MediaFormat::Svg));
        let exported = export_markdown(flow, &opened.presentation.media);
        assert!(exported.contains("requirementDiagram"), "exported:\n{exported}");
        assert!(exported.contains("C4Context"), "exported:\n{exported}");
        assert_eq!(exported.matches("```mermaid").count(), 3, "{exported}");
        let bytes = opened.source.save(&opened.presentation, Default::default()).unwrap();
        let document = crate::read(&bytes).unwrap();
        assert_eq!(export_markdown(document.flow.as_ref().unwrap(), &document.media), exported);

        let slides = open_markdown_presentation(md.as_bytes().to_vec()).unwrap();
        let bytes = slides.source.save(&slides.presentation, Default::default()).unwrap();
        let presentation = crate::read(&bytes).unwrap();
        let exported = export_presentation_markdown(&presentation);
        assert_eq!(exported.matches("```mermaid").count(), 3, "{exported}");
        assert!(exported.contains("requirementDiagram\n  requirement req1"), "{exported}");
    }

    #[test]
    fn pptx_mermaid_fences_keep_source_order_across_shape_kinds() {
        let markdown = r#"# 図の順序

導入

```mermaid
pie
  "承認" : 70
  "保留" : 30
```

```mermaid
flowchart LR
  A[受付] --> B[承認]
```

図の注記

```mermaid
zenuml
  Author.review()
```

```mermaid
xychart horizontal
  x-axis [一月, 二月]
  bar [10, 20]
```

まとめ
"#;
        let slides = open_markdown_presentation(markdown.as_bytes().to_vec()).unwrap();
        let pptx = slides.source.save(&slides.presentation, Default::default()).unwrap();
        let reopened = crate::read(&pptx).unwrap();
        let exported = export_presentation_markdown(&reopened);
        let (_, body) = split_frontmatter_body(exported.strip_prefix("---\n").unwrap()).unwrap();
        let actual: Vec<_> = body.lines().filter(|line| {
            !line.is_empty() && *line != PPTX_SLIDE_MARKER && !line.starts_with(PPTX_SHAPE_MARKER)
        }).collect();
        let expected: Vec<_> = markdown.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(actual, expected, "{exported}");
    }

    #[test]
    fn mermaid_diagrams_roundtrip_svg_and_native_charts() {
        let md = "\
# Mermaid

```mermaid
sequenceDiagram
  User->>API: Request
  API-->>User: Response
```

```mermaid
flowchart LR
  A[Start] --> B{Check}
  B -->|OK| C((Done))
```

```mermaid
pie showData
  title Share
  \"Chrome\" : 65
  \"Other\" : 35
```

```mermaid
xychart-beta
  title Sales
  x-axis [Jan, Feb]
  line [10, 20]
```
";
        let opened = open_markdown(md.as_bytes().to_vec()).expect("open_markdown");
        let pres = &opened.presentation;
        let flow = pres.flow.as_ref().unwrap();

        let svg_count = pres
            .media
            .iter()
            .filter(|m| m.format == MediaFormat::Svg)
            .count();
        assert_eq!(svg_count, 2, "sequenceDiagram and flowchart should produce 2 SVG media items");

        let has_native_chart = flow.body.iter().any(|b| {
            matches!(
                b,
                Block::Paragraph(p)
                    if p.runs.iter().any(|r| matches!(&r.kind, FlowRunKind::Shape(s) if matches!(s.kind, ShapeKind::Chart(_))))
            )
        });
        assert!(has_native_chart, "pie diagram should produce a native Office chart");
        let mut saw_pie_labels = false;
        for block in &flow.body {
            let Block::Paragraph(paragraph) = block else { continue };
            for run in &paragraph.runs {
                let FlowRunKind::Shape(shape) = &run.kind else { continue };
                let ShapeKind::Chart(chart) = &shape.kind else { continue };
                if chart.groups.iter().any(|group| matches!(group.kind, ChartType::Pie { .. })) {
                    saw_pie_labels = chart.groups.iter().flat_map(|group| &group.series).any(|series| {
                        series.labels.as_ref().is_some_and(|labels| labels.show_value && labels.show_percent)
                    });
                }
            }
        }
        assert!(saw_pie_labels, "pie showData should label the slices");

        // Verify .docx save + reread works cleanly
        let saved = opened.source.save(pres, Default::default()).expect("save docx");
        let reread = crate::read(&saved).expect("reread docx");
        assert!(reread.media.iter().any(|m| m.format == MediaFormat::Svg));

        // Verify Markdown export preserves mermaid source for SVG diagrams and chart YAML for native charts
        let exported = export_markdown(flow, &pres.media);
        assert!(exported.contains("```mermaid\nsequenceDiagram"));
        assert!(exported.contains("```mermaid\nflowchart LR"));
        assert!(exported.contains("```mermaid\npie showData"));
        assert!(exported.contains("```mermaid\nxychart-beta"));
        assert!(!exported.contains("```chart\ntype: pie"));
        assert!(!exported.contains("```chart\ntype: line"));

        let saved_md = opened.source.save(pres, Default::default()).expect("save docx for mermaid chart");
        let reread_md = crate::read(&saved_md).expect("reread mermaid chart");
        let reread_flow = reread_md.flow.as_ref().expect("flow");
        let reread_exported = export_markdown(reread_flow, &reread_md.media);
        assert!(reread_exported.contains("```mermaid\npie showData"), "saved docx should keep the pie fence:\n{reread_exported}");
        assert!(reread_exported.contains("```mermaid\nxychart-beta"), "saved docx should keep the line fence:\n{reread_exported}");

        let mut edited = opened.presentation.clone();
        let flow = std::sync::Arc::make_mut(edited.flow.as_mut().expect("flow"));
        let mut changed = false;
        for block in &mut flow.body {
            let Block::Paragraph(paragraph) = block else { continue };
            for run in &mut paragraph.runs {
                let FlowRunKind::Shape(shape) = &mut run.kind else { continue };
                let ShapeKind::Chart(chart) = &mut shape.kind else { continue };
                if let Some(value) = chart.groups.first_mut().and_then(|g| g.series.first_mut()).and_then(|s| s.values.first_mut()) {
                    *value = Some(Number(1.0));
                    changed = true;
                }
            }
        }
        assert!(changed, "pie chart value should be editable");
        let edited_md = export_markdown(edited.flow.as_ref().unwrap(), &edited.media);
        assert!(edited_md.contains("```chart\ntype: pie"), "edited pie should leave Mermaid:\n{edited_md}");
        assert!(edited_md.contains("```chart\ntype: line"), "edited line should leave Mermaid:\n{edited_md}");
        assert!(!edited_md.contains("```mermaid\npie"));
        assert!(!edited_md.contains("```mermaid\nxychart-beta"));
    }

    #[test]
    fn japanese_mermaid_diagrams_survive_docx_and_pptx_save() {
        let diagrams = [
            "flowchart LR\n  A[申請受付] --> B{内容を確認する}\n  B --> C[承認]\n",
            "sequenceDiagram\n  利用者->>窓口: 長い申請内容を送信\n  窓口-->>利用者: 受付完了\n",
            "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  CUSTOMER {\n    string name\n  }\n",
            "gantt\n  dateFormat YYYY-MM-DD\n  section 進捗\n  要件確認 :a, 2026-01-05, 3d\n",
            "pie\n  title 構成比\n  \"承認\" : 70\n  \"保留\" : 30\n",
            "xychart horizontal\n  x-axis [一月, 二月]\n  bar [10, 20]\n",
            "xychart horizontal\n  x-axis [一月, 二月]\n  bar [10, 20]\n  line [7, 15 \"目標\"]\n",
            "xychart\n  x-axis \"経過秒\" 0 --> 10\n  line [10, 20 \"終了\"]\n",
            "xychart-beta\n  title 月別売上\n  x-axis [一月, 二月]\n  line [10, 20]\n",
            "ishikawa-beta\n  受付の遅延\n  手順\n    確認漏れ\n  設備\n    通知が届かない\n",
            "cynefin-beta\n  clear\n    \"手順が明確\"\n  clear --> chaotic : \"状況が変化\"\n",
            "zenuml\n  利用者->窓口.受付() {\n    return 完了\n  }\n",
        ];
        let body = diagrams.iter().map(|diagram| format!("```mermaid\n{diagram}```\n\n")).collect::<String>();
        let doc = open_markdown(body.as_bytes().to_vec()).expect("open document");
        let bytes = doc.source.save(&doc.presentation, Default::default()).expect("save docx");
        let reread = crate::read(&bytes).expect("read docx");
        let exported = export_markdown(reread.flow.as_ref().expect("document flow"), &reread.media);
        for diagram in &diagrams {
            assert!(exported.contains(&format!("```mermaid\n{diagram}```")), "DOCX lost {diagram}:\n{exported}");
        }
        let slides = open_markdown_presentation(format!("# 図\n\n{body}").into_bytes()).expect("open slides");
        let bytes = slides.source.save(&slides.presentation, Default::default()).expect("save pptx");
        let reread = crate::read(&bytes).expect("read pptx");
        let exported = export_presentation_markdown(&reread);
        for diagram in &diagrams {
            assert!(exported.contains(&format!("```mermaid\n{diagram}```")), "PPTX lost {diagram}:\n{exported}");
        }
    }

    #[test]
    fn mermaid_categories_remain_literal_in_office_chart_workbooks() {
        let source = r#"xychart
  x-axis ["Apr 2022", "003", "=1+1", "TRUE"]
  y-axis 0 --> 40
  line [10, 20, 30, 35]
"#;
        let markdown = format!("# 図\n\n```mermaid\n{source}```\n");
        let document = open_markdown(markdown.as_bytes().to_vec()).unwrap();
        let docx = document.source.save(&document.presentation, Default::default()).unwrap();
        let reread = crate::read(&docx).unwrap();
        let exported = export_markdown(reread.flow.as_ref().unwrap(), &reread.media);
        assert!(exported.contains(source), "{exported}");

        let slides = open_markdown_presentation(markdown.into_bytes()).unwrap();
        let pptx = slides.source.save(&slides.presentation, Default::default()).unwrap();
        let reread = crate::read(&pptx).unwrap();
        let exported = export_presentation_markdown(&reread);
        assert!(exported.contains(source), "{exported}");
    }

    #[test]
    fn horizontal_mermaid_bar_stays_native_after_pptx_save() {
        let markdown = b"# Chart\n\n```mermaid\nxychart horizontal\n  x-axis [A, B]\n  y-axis 0 --> 30\n  bar [10, 20]\n```\n";
        let opened = open_markdown_presentation(markdown.to_vec()).expect("open slides");
        let bytes = opened.source.save(&opened.presentation, Default::default()).expect("save pptx");
        let reread = crate::read(&bytes).expect("read pptx");
        assert!(reread.slides[0].shapes.iter().any(|shape| {
            let ShapeKind::Chart(chart) = &shape.kind else { return false };
            chart_value_range(chart) == Some((0.0, 30.0))
                && chart.groups.iter().any(|group| matches!(group.kind, ChartType::Bar { horizontal: true, .. }))
        }));
        assert!(export_presentation_markdown(&reread).contains("```mermaid\nxychart horizontal"));
    }

    #[test]
    fn pptx_markdown_tables_keep_rectangular_merges_and_header_options() {
        let input = r#"---
marp: true
---
# 見出し行なし

| <!-- pptx-no-header --> 結合 <!-- colspan:2 --> <!-- vmerge:start --> | | 一 |
| --- | --- | --- |
| <!-- colspan:2 --> <!-- vmerge:continue --> | | 二 |
| 左 | 中 | 右\|端 |

---

# 見出し行あり

| 見出し | 値 |
| --- | --- |
| 項目 | 12 |
"#;
        let pptx = import_markdown_to_pptx_bytes(input.as_bytes()).unwrap();
        let markdown = export_presentation_markdown(&crate::read(&pptx).unwrap());
        assert!(markdown.contains("<!-- pptx-no-header --> 結合 <!-- colspan:2 --> <!-- vmerge:start -->"));
        assert!(markdown.contains("<!-- colspan:2 --> <!-- vmerge:continue -->"));
        let rebuilt = crate::read(&import_markdown_to_pptx_bytes(markdown.as_bytes()).unwrap()).unwrap();
        let tables = rebuilt.slides.iter().flat_map(|slide| &slide.shapes).filter_map(|shape| {
            if let ShapeKind::Table(table) = &shape.kind { Some(table) } else { None }
        }).collect::<Vec<_>>();
        assert_eq!(tables.len(), 2);
        assert!(!tables[0].style_options.first_row);
        assert!(tables[1].style_options.first_row);
        let rows = &tables[0].rows;
        assert!(rows.iter().all(|row| row.cells.len() == 3));
        assert_eq!((rows[0].cells[0].column_span, rows[0].cells[0].row_span), (2, 2));
        assert!(rows[0].cells[1].merged && rows[1].cells[0].merged && rows[1].cells[1].merged);
        assert!(!rows[1].cells[2].merged);
        let text = rows[2].cells[2].text.paragraphs[0].runs.iter().map(|run| run.text.as_str()).collect::<String>();
        assert_eq!(text, "右|端");
    }

    #[test]
    fn pptx_markdown_keeps_frames_and_stack_when_text_is_edited() {
        let markdown = r#"---
marp: true
pptx_size: [9144000,6858000]
---

<!-- pptx-slide -->

<!-- pptx-shape:{"frame":[100,200,3000000,2200000],"preset":"ellipse","fill":[16711680,255]} -->

<!-- pptx-shape:{"frame":[700000,100000,1500000,1200000],"rotation":120000,"flip_h":true} -->

![Sample](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/a5sAAAAASUVORK5CYII=)

<!-- pptx-shape:{"frame":[900000,600000,3200000,1100000]} -->

Title

Second paragraph

<!-- pptx-shape:{"frame":[1000000,650000,2400000,900000]} -->

Front caption
"#;
        let initial = crate::read(&import_markdown_to_pptx_bytes(markdown.as_bytes()).unwrap()).unwrap();
        let edited = export_presentation_markdown(&initial).replace("Second paragraph", "Edited paragraph");
        let rebuilt = crate::read(&import_markdown_to_pptx_bytes(edited.as_bytes()).unwrap()).unwrap();
        assert_eq!(rebuilt.slide_size, EmuSize::new(9144000, 6858000));
        let shapes = &rebuilt.slides[0].shapes;
        assert_eq!(shapes.len(), 4);
        assert_eq!(shapes[0].frame, EmuRect::new(100, 200, 3000000, 2200000));
        assert!(matches!(&shapes[0].kind, ShapeKind::Auto(shape) if shape.geometry == ibis_doc_model::Geometry::Ellipse));
        assert!(matches!(shapes[1].kind, ShapeKind::Picture(_)));
        assert_eq!(shapes[1].frame, EmuRect::new(700000, 100000, 1500000, 1200000));
        assert_eq!(shapes[1].rotation, 120000);
        assert!(shapes[1].flip_h);
        assert_eq!(shapes[2].frame, EmuRect::new(900000, 600000, 3200000, 1100000));
        assert_eq!(shapes[2].text_body().unwrap().paragraphs[1].runs[0].text, "Edited paragraph");
        assert_eq!(shapes[3].frame, EmuRect::new(1000000, 650000, 2400000, 900000));
        assert_eq!(shapes[3].text_body().unwrap().paragraphs[0].runs[0].text, "Front caption");
    }

    #[test]
    fn markdown_presentation_roundtrips_slides_mermaid_charts_and_tables() {
        let md = "\
# AI 連携ロードマップ
- Markdown ネイティブ対応
- pure-Rust Mermaid

<!-- notes: 表紙の補足メモ -->

---

# アーキテクチャ

```mermaid
flowchart LR
  MD[Markdown] --> Model[DocModel]
  Model --> PPTX[PPTX]
```

---

# 比較表とグラフ

| 項目 | 状況 |
| :--- | :--- |
| DOCX | 対応済み |
| PPTX | 対応済み |

```mermaid
pie
  title 構成比
  \"Rust\" : 100
```

```mermaid
xychart-beta
  title 売上
  x-axis [1月, 2月]
  line [3, 5]
```
";
        let opened = open_markdown_presentation(md.as_bytes().to_vec()).expect("open_markdown_presentation");
        let pres = &opened.presentation;
        assert_eq!(pres.slides.len(), 3, "expected 3 slides");
        assert!(pres.media.iter().any(|m| m.format == MediaFormat::Svg), "expected SVG from flowchart");

        let saved = opened.source.save(pres, Default::default()).expect("save pptx");
        let reread = crate::read(&saved).expect("reread pptx");
        assert_eq!(reread.slides.len(), 3);

        let exported = export_presentation_markdown(&reread);
        assert!(exported.contains("# AI 連携ロードマップ"), "exported:\n{exported}");
        assert!(exported.contains("---"));
        assert!(exported.contains("```mermaid\nflowchart LR"));
        assert!(exported.contains("| DOCX | 対応済み |"));
        assert!(exported.contains("```mermaid\npie"), "saved pptx should keep the pie fence:\n{exported}");
        assert!(exported.contains("```mermaid\nxychart-beta"), "saved pptx should keep the line fence:\n{exported}");
        assert!(!exported.contains("```chart\ntype: pie"));
        assert!(!exported.contains("```chart\ntype: line"));
    }

    #[test]
    fn mermaid_xychart_y_axis_range_reaches_native_chart_and_survives_docx() {
        let md = "\
```mermaid
xychart
  title Revenue
  x-axis [Jan, Feb, Mar]
  y-axis \"円\" 4000 --> 11000
  bar [5000, 6000, 7500]
```
";
        let value_range = |flow: &FlowDocument| -> Vec<(Option<f64>, Option<f64>)> {
            let mut out = Vec::new();
            for block in &flow.body {
                let Block::Paragraph(paragraph) = block else { continue };
                for run in &paragraph.runs {
                    let FlowRunKind::Shape(shape) = &run.kind else { continue };
                    let ShapeKind::Chart(chart) = &shape.kind else { continue };
                    for axis in chart.axes.iter().filter(|axis| axis.kind == AxisKind::Value) {
                        out.push((axis.min.map(|n| n.0), axis.max.map(|n| n.0)));
                    }
                }
            }
            out
        };
        let opened = open_markdown(md.as_bytes().to_vec()).expect("open_markdown");
        let pres = &opened.presentation;
        let flow = pres.flow.as_ref().expect("flow");
        assert_eq!(value_range(flow), vec![(Some(4000.0), Some(11000.0))]);
        let exported = export_markdown(flow, &pres.media);
        assert!(exported.contains("```mermaid\nxychart\n"), "{exported}");
        assert!(exported.contains("4000 --> 11000"), "{exported}");

        let saved = opened.source.save(pres, Default::default()).expect("save docx");
        let reread = crate::read(&saved).expect("reread docx");
        let reread_flow = reread.flow.as_ref().expect("flow");
        assert_eq!(value_range(reread_flow), vec![(Some(4000.0), Some(11000.0))]);
        let reread_md = export_markdown(reread_flow, &reread.media);
        assert!(reread_md.contains("4000 --> 11000"), "saved docx keeps the fence:\n{reread_md}");

        // Editing the axis range makes the fence stale, so the chart is written as a ```chart block.
        let mut edited = opened.presentation.clone();
        let flow = std::sync::Arc::make_mut(edited.flow.as_mut().expect("flow"));
        for block in &mut flow.body {
            let Block::Paragraph(paragraph) = block else { continue };
            for run in &mut paragraph.runs {
                let FlowRunKind::Shape(shape) = &mut run.kind else { continue };
                let ShapeKind::Chart(chart) = &mut shape.kind else { continue };
                for axis in chart.axes.iter_mut().filter(|axis| axis.kind == AxisKind::Value) {
                    axis.max = Some(Number(20000.0));
                }
            }
        }
        let edited_md = export_markdown(edited.flow.as_ref().unwrap(), &edited.media);
        assert!(!edited_md.contains("4000 --> 11000"), "{edited_md}");
    }

    #[test]
    fn mermaid_pie_legend_survives_office_roundtrip() {
        for (name, side, position) in [
            ("top", Side::Top, LegendPosition::Top),
            ("bottom", Side::Bottom, LegendPosition::Bottom),
            ("left", Side::Left, LegendPosition::Left),
            ("right", Side::Right, LegendPosition::Right),
        ] {
            let md = format!("```mermaid\n---\nconfig:\n  pie:\n    legendPosition: {name}\n---\npie\n  \"A\" : 1\n  \"B\" : 3\n```\n");
            let opened = open_markdown(md.as_bytes().to_vec()).unwrap();
            let saved = opened.source.save(&opened.presentation, Default::default()).unwrap();
            let reread = crate::read(&saved).unwrap();
            for pres in [&opened.presentation, &reread] {
                let flow = pres.flow.as_ref().unwrap();
                let Block::Paragraph(paragraph) = &flow.body[0] else { panic!("expected chart paragraph") };
                let FlowRunKind::Shape(shape) = &paragraph.runs[0].kind else { panic!("expected chart shape") };
                let ShapeKind::Chart(chart) = &shape.kind else { panic!("expected native chart") };
                let legend = chart.legend.as_ref().unwrap();
                assert_eq!(legend.side, side);
                assert_eq!(legend.source_position, Some(position));
                assert!(export_markdown(flow, &pres.media).contains(&format!("legendPosition: {name}")));
            }

            let deck = open_markdown_presentation(md.into_bytes()).unwrap();
            let saved = deck.source.save(&deck.presentation, Default::default()).unwrap();
            let reread = crate::read(&saved).unwrap();
            for pres in [&deck.presentation, &reread] {
                let chart = pres.slides.iter().flat_map(|slide| &slide.shapes).find_map(|shape| {
                    match &shape.kind {
                        ShapeKind::Chart(chart) => Some(chart),
                        _ => None,
                    }
                }).expect("native pie chart");
                let legend = chart.legend.as_ref().unwrap();
                assert_eq!(legend.side, side);
                assert_eq!(legend.source_position, Some(position));
            }
        }
    }

    #[test]
    fn mermaid_chart_config_reaches_native_charts() {
        let md = "\
```mermaid
---
config:
  pie:
    donutHole: 0.4
    highlightSlice: B
---
pie
  \"A\" : 1
  \"B\" : 3
```

```mermaid
---
config:
  xyChart:
    showDataLabel: true
  themeVariables:
    xyChart:
      plotColorPalette: '#ff0000, #00ff00'
---
xychart
  x-axis [a, b]
  bar [1, 2]
  line [2, 3]
```
";
        let opened = open_markdown(md.as_bytes().to_vec()).expect("open_markdown");
        let flow = opened.presentation.flow.as_ref().expect("flow");
        let charts: Vec<&Chart> = flow
            .body
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(p) => Some(p),
                _ => None,
            })
            .flat_map(|p| p.runs.iter())
            .filter_map(|run| match &run.kind {
                FlowRunKind::Shape(shape) => match &shape.kind {
                    ShapeKind::Chart(chart) => Some(&**chart),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(charts.len(), 2);
        let pie = &charts[0].groups[0];
        assert!(matches!(pie.kind, ChartType::Doughnut { hole: 40, .. }), "{:?}", pie.kind);
        assert_eq!(pie.series[0].point_explosions, vec![(1, 15)]);
        let xy = charts[1];
        let series: Vec<&Series> = xy.groups.iter().flat_map(|g| g.series.iter()).collect();
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].fill, Fill::Solid(ibis_doc_model::Color::opaque(Rgb(0xff0000))));
        assert_eq!(series[1].stroke.as_ref().map(|s| s.color), Some(ibis_doc_model::Color::opaque(Rgb(0x00ff00))));
        assert!(series.iter().all(|s| s.labels.as_ref().is_some_and(|l| l.show_value)));
        // The fence (with its config) is still what is exported.
        let exported = export_markdown(flow, &opened.presentation.media);
        assert!(exported.contains("donutHole: 0.4") && exported.contains("plotColorPalette"), "{exported}");
    }

    #[test]
    fn mermaid_phase2_flowchart_fence_and_combo_chart_roundtrip() {
        let md = "\
```mermaid
flowchart TD
  A[\"一行<br>二行\"] --> B[(筒)]
```

```mermaid
xychart-beta
  title Mix
  x-axis [Jan, Feb]
  bar \"売上\" [10, 20]
  line \"目標\" [8, 18]
```
";
        let opened = open_markdown(md.as_bytes().to_vec()).expect("open_markdown");
        let pres = &opened.presentation;
        let flow = pres.flow.as_ref().expect("flow");
        let exported = export_markdown(flow, &pres.media);
        assert!(exported.contains("```mermaid\nflowchart TD"), "{exported}");
        assert!(exported.contains("一行<br>二行"), "{exported}");
        assert!(exported.contains("[(筒)]"), "{exported}");
        assert!(exported.contains("```mermaid\nxychart-beta"), "{exported}");

        let mut saw_combo = false;
        for block in &flow.body {
            let Block::Paragraph(paragraph) = block else { continue };
            for run in &paragraph.runs {
                let FlowRunKind::Shape(shape) = &run.kind else { continue };
                let ShapeKind::Chart(chart) = &shape.kind else { continue };
                let kinds: Vec<_> = chart.groups.iter().map(|group| group.kind).collect();
                assert!(
                    matches!(kinds.as_slice(), [ChartType::Bar { horizontal: false, .. }, ChartType::Line]),
                    "combo groups: {kinds:?}"
                );
                let names: Vec<_> = chart.groups.iter().flat_map(|group| &group.series).map(|series| series.name.as_str()).collect();
                assert_eq!(names, vec!["売上", "目標"]);
                saw_combo = true;
            }
        }
        assert!(saw_combo, "expected a bar+line chart");

        let saved = opened.source.save(pres, Default::default()).expect("save combo docx");
        let reread = crate::read(&saved).expect("reread combo docx");
        let reread_flow = reread.flow.as_ref().expect("flow");
        let reread_md = export_markdown(reread_flow, &reread.media);
        assert!(reread_md.contains("```mermaid\nxychart-beta"), "saved combo should keep the fence:\n{reread_md}");
        assert!(reread_md.contains("```mermaid\nflowchart TD"), "{reread_md}");
        let mut reread_combo = false;
        for block in &reread_flow.body {
            let Block::Paragraph(paragraph) = block else { continue };
            for run in &paragraph.runs {
                let FlowRunKind::Shape(shape) = &run.kind else { continue };
                let ShapeKind::Chart(chart) = &shape.kind else { continue };
                assert!(chart.groups.iter().any(|group| matches!(group.kind, ChartType::Bar { .. })));
                assert!(chart.groups.iter().any(|group| matches!(group.kind, ChartType::Line)));
                reread_combo = true;
            }
        }
        assert!(reread_combo, "reread docx should keep the combo groups");

        let slides = "\
# 複合

```mermaid
xychart-beta
  title Mix
  x-axis [Jan, Feb]
  bar \"売上\" [10, 20]
  line \"目標\" [8, 18]
```
";
        let opened = open_markdown_presentation(slides.as_bytes().to_vec()).expect("presentation");
        let saved = opened.source.save(&opened.presentation, Default::default()).expect("save combo pptx");
        let reread = crate::read(&saved).expect("reread combo pptx");
        let exported = export_presentation_markdown(&reread);
        assert!(exported.contains("```mermaid\nxychart-beta"), "{exported}");
        let slide_combo = reread.slides.iter().flat_map(|slide| &slide.shapes).any(|shape| {
            let ShapeKind::Chart(chart) = &shape.kind else { return false };
            chart.groups.iter().any(|group| matches!(group.kind, ChartType::Bar { .. }))
                && chart.groups.iter().any(|group| matches!(group.kind, ChartType::Line))
        });
        assert!(slide_combo, "slide copy should keep bar and line groups");
    }

    #[test]
    fn exports_workbook_sheets_to_markdown_tables() {
        let pres = crate::save::open(b"Name,Score,City\nAlice,95,Tokyo\nBob,88,Osaka\n".to_vec())
            .expect("open csv")
            .presentation;
        let md = export_presentation_to_markdown(&pres).expect("export workbook to markdown");
        assert!(md.contains("| Name | Score | City |"));
        assert!(md.contains("| Alice | 95 | Tokyo |"));
        assert!(md.contains("| Bob | 88 | Osaka |"));
    }

    #[test]
    fn markdown_workbook_roundtrips_tables_to_xlsx_and_back() {
        let md = "\
| Product | Qty | Price |
| :--- | ---: | ---: |
| Apple | 10 | 120 |
| Orange | 5 | 80 |
";
        let opened = open_markdown_workbook(md.as_bytes().to_vec()).expect("open_markdown_workbook");
        let saved = opened
            .source
            .save(&opened.presentation, Default::default())
            .expect("save xlsx");
        let reread = crate::read(&saved).expect("reread xlsx");
        let exported = export_presentation_to_markdown(&reread).expect("export xlsx to markdown");
        assert!(exported.contains("| Product | Qty | Price |"));
        assert!(exported.contains("| Apple | 10 | 120 |"));
        assert!(exported.contains("| Orange | 5 | 80 |"));
    }

    #[test]
    fn named_markdown_sheets_round_trip_in_order() {
        let input = r#"<!-- xlsx-sheet "売上" -->

## 売上

| 区分 | 金額 |
| --- | --- |
| 国内 | 42 |

<!-- xlsx-sheet "Sheet1" -->

## Sheet1

| 区分 | 件数 |
| --- | --- |
| 海外 | 5 |

<!-- xlsx-sheet "空白" -->

## 空白
"#;
        let bytes = import_markdown_to_xlsx_bytes(input.as_bytes()).unwrap();
        let workbook = crate::read(&bytes).unwrap();
        let sheets = &workbook.workbook.as_ref().unwrap().sheets;
        assert_eq!(sheets.iter().map(|sheet| sheet.name.as_str()).collect::<Vec<_>>(), ["売上", "Sheet1", "空白"]);
        for (sheet, value) in sheets.iter().zip(["42", "5"]) {
            let slide = workbook.slide(sheet.slide.unwrap()).unwrap();
            let contents = ibis_doc_model::sheet_calc::sheet_of_slide(slide).unwrap();
            assert_eq!(contents.cell(1, 1).unwrap().text, value);
        }
        let markdown = export_workbook_markdown(&workbook).unwrap();
        assert!(markdown.contains(r#"<!-- xlsx-sheet "空白" -->"#));
        let rebuilt = import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap();
        assert_eq!(export_workbook_markdown(&crate::read(&rebuilt).unwrap()).unwrap(), markdown);
        assert!(import_markdown_to_xlsx_bytes(input.replace("空白", "売上").as_bytes()).is_err());
    }

    #[test]
    fn workbook_sheet_origin_retains_coordinates_and_empty_rows() {
        let input = br#"<!-- xlsx-sheet {"name":"Offset","origin":"B3"} -->

| Code | Amount |
| --- | --- |
| | |
| Local | 42 |
"#;
        let first = import_markdown_to_xlsx_bytes(input).unwrap();
        let markdown = export_workbook_markdown(&crate::read(&first).unwrap()).unwrap();
        assert!(markdown.contains(r#"<!-- xlsx-sheet {"name":"Offset","origin":"B3"} -->"#));
        let rebuilt = crate::read(&import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap()).unwrap();
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
        assert_eq!(sheet.cell(2, 1).unwrap().text, "Code");
        assert!(sheet.cell(3, 1).is_none());
        assert_eq!(sheet.cell(4, 2).unwrap().text, "42");
        for origin in ["XFE3", "XFD3", "B1048576", "B0"] {
            let invalid = std::str::from_utf8(input).unwrap().replace("B3", origin);
            assert!(import_markdown_to_xlsx_bytes(invalid.as_bytes()).is_err(), "{origin}");
        }
    }

    #[test]
    fn workbook_ambiguous_text_cells_keep_their_type() {
        use ibis_doc_model::CellValue;
        let input = br#"<!-- xlsx-sheet {"name":"Types","origin":"B3"} -->

| Code | Count | Label | Flag |
| --- | --- | --- | --- |
| 0012 <!-- xlsx-text --> | 12 | TRUE <!-- xlsx-text --> | TRUE |
| 1,200 <!-- xlsx-text --> | 1200 | FALSE <!-- xlsx-text --> | FALSE |
"#;
        let bytes = import_markdown_to_xlsx_bytes(input).unwrap();
        let markdown = export_workbook_markdown(&crate::read(&bytes).unwrap()).unwrap();
        assert!(markdown.contains("| 0012 <!-- xlsx-text --> | 12 | TRUE <!-- xlsx-text --> | TRUE |"));
        let rebuilt = crate::read(&import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap()).unwrap();
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
        for (row, expected) in [
            (3, [CellValue::Text("0012".into()), CellValue::Number(12.0), CellValue::Text("TRUE".into()), CellValue::Bool(true)]),
            (4, [CellValue::Text("1,200".into()), CellValue::Number(1200.0), CellValue::Text("FALSE".into()), CellValue::Bool(false)]),
        ] {
            for (column, value) in (1..).zip(expected) {
                assert_eq!(sheet.cell(row, column).unwrap().value, value);
            }
        }
    }

    #[test]
    fn workbook_number_formats_and_date_system_round_trip() {
        let body = r##"<!-- xlsx-sheet "Formats" -->

| Percentage | Currency | Grouping | Date | Custom |
| --- | --- | --- | --- | --- |
| 25% <!-- xlsx-number:0.25 --> <!-- xlsx-format:"0%" --> | \$1,234.50 <!-- xlsx-number:1234.5 --> <!-- xlsx-format:"$#,##0.00" --> | 1,000 <!-- xlsx-format:"#,##0" --> <!-- xlsx-formula:"=500*2" --> | 2025-01-01 <!-- xlsx-number:45000 --> <!-- xlsx-format:"yyyy-mm-dd" --> | 5 <!-- xlsx-format:"0\u0022a\u007cb-\u002dc\u0022" --> |
"##;
        for date1904 in [false, true] {
            let input = if date1904 { format!("{XLSX_1904_MARKER}\n{body}") } else { body.to_owned() };
            let first = crate::read(&import_markdown_to_xlsx_bytes(input.as_bytes()).unwrap()).unwrap();
            let markdown = export_workbook_markdown(&first).unwrap();
            assert_eq!(markdown.starts_with(XLSX_1904_MARKER), date1904);
            let rebuilt = crate::read(&import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap()).unwrap();
            assert_eq!(rebuilt.workbook.as_ref().unwrap().date1904, date1904);
            let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
            for (column, (value, format)) in [
                (0.25, "0%"),
                (1234.5, "$#,##0.00"),
                (1000.0, "#,##0"),
                (45000.0, "yyyy-mm-dd"),
                (5.0, r#"0"a|b--c""#),
            ].into_iter().enumerate() {
                let cell = sheet.cell(1, column as u32).unwrap();
                assert_eq!(cell.value, ibis_doc_model::CellValue::Number(value));
                assert_eq!(sheet.styles[cell.style as usize].number_format, format);
            }
            assert_eq!(sheet.cell(1, 0).unwrap().text, "25%");
            assert_eq!(sheet.cell(1, 2).unwrap().formula.as_deref(), Some("500*2"));
            assert_eq!(sheet.cell(1, 3).unwrap().text, if date1904 { "2027-03-16" } else { "2023-03-15" });
        }
    }

    #[test]
    fn formatted_workbook_numbers_keep_their_stored_values() {
        use ibis_doc_model::CellValue;
        let bytes = import_markdown_to_xlsx_bytes(br#"<!-- xlsx-sheet "Numbers" -->

| Percentage | Rounded | Tiny | Plain |
| --- | --- | --- | --- |
| 0.25 | 1.234 | 1e-8 | 12 |
"#).unwrap();
        let mut workbook = crate::read(&bytes).unwrap();
        let ShapeKind::Sheet(sheet) = &mut workbook.slides[0].shapes[0].kind else {
            panic!("expected a sheet");
        };
        let sheet = Arc::make_mut(sheet);
        let mut cells = sheet.cells.to_vec();
        for (cell, display) in cells.iter_mut().filter(|cell| cell.row == 1)
            .zip(["25%", "1.23", "0.00", "12"])
        {
            cell.text = display.into();
        }
        sheet.cells = cells.into();

        let markdown = export_workbook_markdown(&workbook).unwrap();
        assert!(markdown.contains("| 25% <!-- xlsx-number:0.25 --> | 1.23 <!-- xlsx-number:1.234 --> | 0.00 <!-- xlsx-number:0.00000001 --> | 12 |"));
        let rebuilt = crate::read(&import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap()).unwrap();
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
        for (column, value) in (0..).zip([0.25, 1.234, 1e-8, 12.0]) {
            assert_eq!(sheet.cell(1, column).unwrap().value, CellValue::Number(value));
        }
        assert!(import_markdown_to_xlsx_bytes(markdown.replace("xlsx-number:0.25", "xlsx-number:NaN").as_bytes()).is_err());
    }

    #[test]
    fn workbook_hidden_cells_leave_empty_slots_without_exporting_their_contents() {
        let bytes = import_markdown_to_xlsx_bytes(br#"<!-- xlsx-sheet {"name":"Visible","origin":"B3"} -->

| H1 | HIDDEN_COL | H3 |
| --- | --- | --- |
| HIDDEN_ROW | HIDDEN_BOTH | HIDDEN_ROW |
| Left | HIDDEN_COL | Right |
"#).unwrap();
        let mut workbook = crate::read(&bytes).unwrap();
        let ShapeKind::Sheet(sheet) = &mut workbook.slides[0].shapes[0].kind else {
            panic!("expected a sheet");
        };
        let sheet = Arc::make_mut(sheet);
        sheet.hidden_rows.insert(3, sheet.default_row);
        sheet.hidden_columns.insert(2, sheet.default_column);

        sheet.hidden_rows.insert(7, sheet.default_row);
        sheet.hidden_rows.insert(8, sheet.default_row);
        sheet.validations.push(ibis_doc_model::Validation {
            ranges: vec![
                ibis_doc_model::CellRange {
                    first_row: 2, first_column: 1, last_row: 4, last_column: 3,
                },
                ibis_doc_model::CellRange {
                    first_row: 9, first_column: 4, last_row: 9, last_column: 4,
                },
            ],
            formula1: Some("PRIVATE_VALIDATION_EXPRESSION".into()),
            ..Default::default()
        });
        let book = Arc::make_mut(workbook.workbook.as_mut().unwrap());
        let mut hidden = book.sheets[0].clone();
        hidden.name = "非表示".into();
        hidden.state = ibis_doc_model::SheetState::VeryHidden;
        book.sheets.push(hidden);

        let ai = export_ai_workbook(&workbook).unwrap();
        assert_eq!(ai["diagnostics"].as_array().unwrap()[2..], [
            serde_json::json!({"code": "hidden_rows_omitted", "source": {"sheet": "Visible", "ranges": ["4:4", "8:9"]}}),
            serde_json::json!({"code": "hidden_columns_omitted", "source": {"sheet": "Visible", "ranges": ["C:C"]}}),
            serde_json::json!({"code": "data_validation_omitted", "source": {"sheet": "Visible", "ranges": ["B3:D5", "E10"]}}),
            serde_json::json!({"code": "hidden_sheet_omitted", "source": {"sheet": "非表示"}}),
        ]);
        assert!(!ai.to_string().contains("PRIVATE_VALIDATION_EXPRESSION"));
        let markdown = ai["markdown"].as_str().unwrap();
        assert!(!markdown.contains("HIDDEN"));
        assert!(markdown.contains("| H1 |  | H3 |"));
        assert!(markdown.contains("|  |  |  |"));
        assert!(markdown.contains("| Left |  | Right |"));
        assert!(ai["cells"].as_array().unwrap().iter().all(|cell|
            cell["source"]["row"] != 4 && cell["source"]["column"] != 3
        ));

        let rebuilt = crate::read(&import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap()).unwrap();
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
        assert_eq!(sheet.cell(4, 3).unwrap().text, "Right");
        assert!(sheet.cell(3, 1).is_none());
        assert!(sheet.cell(4, 2).is_none());
    }

    #[test]
    fn workbook_defined_names_preserve_scope_and_relative_formulas() {
        let input = r#"<!-- xlsx-date-system:1904 -->
<!-- xlsx-defined-names [{"name":"Choices","formula":"'選択肢'!$A$1:$A$2"},{"name":"LocalName","sheet":"入力","formula":"A1"},{"name":"InternalValue","sheet":"選択肢","formula":"17","hidden":true},{"name":"Label","formula":"\"A-->B\""}] -->

<!-- xlsx-sheet {"name":"選択肢","state":"hidden"} -->

| 内製 |
| --- |
| 外注 |

<!-- xlsx-sheet {"name":"入力","validations":[{"ranges":["B1"],"type":"list","operator":"between","formula1":"Choices","allowBlank":true,"dropdown":true}]} -->

| 名前付き参照 |
| --- |
| 値 <!-- xlsx-formula:"=LocalName" --> |
"#;
        let bytes = import_markdown_to_xlsx_bytes(input.as_bytes()).unwrap();
        let original = crate::read(&bytes).unwrap();
        let book = original.workbook.as_deref().unwrap();
        assert!(book.date1904);
        assert_eq!(book.names.len(), 4);
        assert_eq!(book.names[1].sheet, Some(1));
        assert_eq!(book.names[1].formula, "A1");
        assert_eq!(book.names[2].sheet, Some(0));
        assert!(book.names[2].hidden);
        let ai = export_ai_source(&bytes).unwrap();
        assert!(!ai.to_string().contains("InternalValue"));
        assert!(ai["diagnostics"].as_array().unwrap().contains(
            &serde_json::json!({"code": "defined_names_omitted", "count": 4}),
        ));

        let options = WorkbookMarkdownOptions { include_defined_names: true, ..Default::default() };
        let partial = export_workbook_markdown_with_options(&original, options).unwrap();
        let partial = crate::read(&import_markdown_to_xlsx_bytes(partial.as_bytes()).unwrap()).unwrap();
        let scoped = &partial.workbook.as_deref().unwrap().names;
        assert_eq!(scoped.len(), 3);
        assert_eq!(scoped[1].sheet, Some(0));
        assert_eq!(scoped[1].formula, "A1");
        assert!(workbook_markdown_omissions(&original, options).contains(
            &serde_json::json!({"code": "defined_names_omitted", "count": 1}),
        ));

        let options = WorkbookMarkdownOptions { include_hidden_sheets: true, include_validations: true, ..options };
        let markdown = export_workbook_markdown_with_options(&original, options).unwrap();
        assert!(workbook_markdown_omissions(&original, options).is_empty());
        assert!(markdown.contains(r"A-\u002d\u003eB"));
        let rebuilt = crate::read(&import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap()).unwrap();
        assert_eq!(rebuilt.workbook.as_deref().unwrap().names, book.names);
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
        assert_eq!(sheet.validations[0].formula1.as_deref(), Some("Choices"));
        assert!(import_markdown_to_xlsx_bytes(input.replace(r#""sheet":"入力""#, r#""sheet":"存在しない""#).as_bytes()).is_err());
        assert!(import_markdown_to_xlsx_bytes(input.replace(r#""name":"Label""#, r#""name":"choices""#).as_bytes()).is_err());
    }

    #[test]
    fn workbook_hidden_sheets_round_trip_with_explicit_markdown_export() {
        use ibis_doc_model::SheetState;

        let input = r#"<!-- xlsx-sheet {"name":"内訳","state":"hidden","origin":"C4","hiddenRows":["5:5"]} -->

| 10 <!-- xlsx-number:10 --> |
| --- |
| PRIVATE_ROW |

<!-- xlsx-sheet "集計" -->

| 10 <!-- xlsx-formula:"=内訳!C4" --> |
| --- |

<!-- xlsx-sheet {"name":"保管","state":"veryHidden"} -->

| PRIVATE_SHEET |
| --- |
"#;
        let opened = open_markdown_workbook(input.as_bytes().to_vec()).unwrap();
        let book = opened.presentation.workbook.as_deref().unwrap();
        assert_eq!(book.active_tab, 1);
        assert_eq!(opened.presentation.slides.len(), 1);
        assert_eq!(book.sheets.iter().map(|sheet| (&*sheet.name, sheet.state)).collect::<Vec<_>>(), [
            ("内訳", SheetState::Hidden), ("集計", SheetState::Visible), ("保管", SheetState::VeryHidden),
        ]);
        let bytes = opened.source.save(&opened.presentation, SaveOptions::default()).unwrap();
        let ai = export_ai_source(&bytes).unwrap();
        assert!(!ai.to_string().contains("PRIVATE"));
        assert!(!ai["markdown"].as_str().unwrap().contains(r#""state":"hidden""#));

        let options = WorkbookMarkdownOptions { include_hidden_sheets: true, ..Default::default() };
        let partial = export_workbook_markdown_with_options(&opened.presentation, options).unwrap();
        assert!(!partial.contains("PRIVATE_ROW"));
        assert!(partial.contains("PRIVATE_SHEET"));
        let diagnostics = workbook_markdown_omissions(&opened.presentation, options);
        assert_eq!(diagnostics, [serde_json::json!({
            "code": "hidden_rows_omitted", "source": {"sheet": "内訳", "ranges": ["5:5"]},
        })]);
        let options = WorkbookMarkdownOptions { include_hidden_axes: true, ..options };
        let markdown = export_workbook_markdown_with_options(&opened.presentation, options).unwrap();
        assert!(workbook_markdown_omissions(&opened.presentation, options).is_empty());
        assert!(markdown.contains("PRIVATE_ROW"));
        let edited = markdown.replace("PRIVATE_SHEET", "編集済み");
        let bytes = import_markdown_to_xlsx_bytes(edited.as_bytes()).unwrap();
        let rebuilt = crate::read(&bytes).unwrap();
        let book = rebuilt.workbook.as_deref().unwrap();
        assert_eq!(book.active_tab, 1);
        assert_eq!(book.sheets[0].state, SheetState::Hidden);
        assert!(book.sheets[0].hidden.as_deref().unwrap().hidden_rows.contains_key(&4));
        assert_eq!(book.sheets[2].state, SheetState::VeryHidden);
        assert_eq!(book.sheets[2].hidden.as_deref().unwrap().cell(0, 0).unwrap().text, "編集済み");
        let summary = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
        assert_eq!(summary.cell(0, 0).unwrap().formula.as_deref(), Some("内訳!C4"));
        assert!(import_markdown_to_xlsx_bytes(br#"<!-- xlsx-sheet {"name":"Sheet1","state":"hidden"} -->"#).is_err());
        assert!(import_markdown_to_xlsx_bytes(br#"<!-- xlsx-sheet {"name":"Sheet1","state":"private"} -->"#).is_err());
    }

    #[test]
    fn workbook_hidden_axes_round_trip_with_explicit_markdown_export() {
        let input = r#"<!-- xlsx-sheet {"name":"入力","origin":"B3","hiddenRows":["4:4","10000:10001"],"hiddenColumns":["C:C","XFC:XFD"]} -->

| Key | HIDDEN_COLUMN | Visible |
| --- | --- | --- |
| HIDDEN_ROW | HIDDEN_FORMULA <!-- xlsx-formula:"=1+2" --> | HIDDEN_ROW |
| Key | HIDDEN_COLUMN | Visible |

<!-- xlsx-sheet {"name":"空","hiddenRows":["9:9"],"hiddenColumns":["F:F"]} -->
"#;
        let bytes = import_markdown_to_xlsx_bytes(input.as_bytes()).unwrap();
        let original = crate::read(&bytes).unwrap();
        let ai = export_ai_source(&bytes).unwrap();
        assert!(!ai.to_string().contains("HIDDEN"));
        assert_eq!(ai["diagnostics"].as_array().unwrap().len(), 6);
        let options = WorkbookMarkdownOptions { include_hidden_axes: true, ..Default::default() };
        assert!(workbook_markdown_omissions(&original, options).is_empty());
        let markdown = export_workbook_markdown_with_options(&original, options).unwrap();
        assert!(markdown.contains(r#""hiddenRows":["4:4","10000:10001"]"#));
        assert!(markdown.contains(r#""hiddenColumns":["C:C","XFC:XFD"]"#));
        assert!(markdown.contains("HIDDEN_ROW"));
        assert!(markdown.contains("HIDDEN_COLUMN"));
        let bytes = import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap();
        let rebuilt = crate::read(&bytes).unwrap();
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
        assert_eq!(sheet.hidden_rows.keys().copied().collect::<Vec<_>>(), [3, 9999, 10000]);
        assert_eq!(sheet.hidden_columns.keys().copied().collect::<Vec<_>>(), [2, 16382, 16383]);
        assert_eq!(sheet.cell(3, 1).unwrap().text, "HIDDEN_ROW");
        assert_eq!(sheet.cell(2, 2).unwrap().text, "HIDDEN_COLUMN");
        assert_eq!(sheet.cell(3, 2).unwrap().formula.as_deref(), Some("1+2"));
        let empty = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[1]).unwrap();
        assert!(empty.cells.is_empty());
        assert!(empty.hidden_rows.contains_key(&8));
        assert!(empty.hidden_columns.contains_key(&5));
        assert!(import_markdown_to_xlsx_bytes(input.replace("XFC:XFD", "XFC:XFE").as_bytes()).is_err());
        assert!(import_markdown_to_xlsx_bytes(input.replace("10000:10001", "4:10001").as_bytes()).is_err());
    }

    #[test]
    fn workbook_validation_rules_round_trip_with_explicit_markdown_export() {
        let list = serde_json::json!({
            "ranges": ["C4:C7", "E9", "G1:G1048576"],
            "type": "list", "operator": "between",
            "formula1": "\"内製,外注\"", "allowBlank": true, "dropdown": false,
            "prompt": ["区分", "選択してください -->"],
            "error": ["warning", "確認", "一覧から選択してください"],
        });
        let number = serde_json::json!({
            "ranges": ["J2:J4"], "type": "whole", "operator": "notBetween",
            "formula1": "1", "formula2": "10", "allowBlank": false, "dropdown": true,
        });
        let first = serde_json::json!({"name": "入力", "origin": "C3", "validations": [list]});
        let second = serde_json::json!({"name": "予備", "validations": [number]});
        let source = format!(
            "<!-- xlsx-sheet {} -->\n\n| 見出し |\n| --- |\n| 内製 |\n\n<!-- xlsx-sheet {second} -->\n",
            first.to_string().replace("--", r"-\u002d"),
        );

        let bytes = import_markdown_to_xlsx_bytes(source.as_bytes()).unwrap();
        let original = crate::read(&bytes).unwrap();
        let rules = |workbook: &Presentation| {
            workbook.slides.iter().map(|slide| {
                ibis_doc_model::sheet_calc::sheet_of_slide(slide).unwrap().validations.clone()
            }).collect::<Vec<_>>()
        };
        assert_eq!(rules(&original), vec![
            vec![parse_markdown_validation(&list).unwrap()],
            vec![parse_markdown_validation(&number).unwrap()],
        ]);

        let plain = export_workbook_markdown(&original).unwrap();
        assert!(!plain.contains("validations"));
        assert!(!plain.contains("一覧から選択してください"));
        let options = WorkbookMarkdownOptions { include_validations: true, ..Default::default() };
        assert_eq!(workbook_markdown_omissions(&original, Default::default()).len(), 2);
        assert!(workbook_markdown_omissions(&original, options).is_empty());
        let ai = export_ai_source(&bytes).unwrap();
        assert_eq!(ai["markdown"], plain);
        assert!(!ai.to_string().contains("一覧から選択してください"));

        let exported = export_workbook_markdown_with_options(&original, options).unwrap();
        assert!(exported.contains(r"選択してください -\u002d\u003e"));
        let edited = exported.replace("| 内製 |", "| 外注 |");
        let rebuilt = crate::read(&import_markdown_to_xlsx_bytes(edited.as_bytes()).unwrap()).unwrap();
        assert_eq!(rules(&rebuilt), rules(&original));
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
        assert_eq!(sheet.cell(3, 2).unwrap().text, "外注");
        assert!(import_markdown_to_xlsx_bytes(exported.replace("G1:G1048576", "G1:G1048577").as_bytes()).is_err());
    }

    #[test]
    fn workbook_formulas_round_trip_and_recalculate_across_sheets() {
        use ibis_doc_model::{CellValue, sheet_calc::{BookView, sheet_of_slide}};
        let input = r#"<!-- xlsx-sheet {"name":"売上","origin":"C3"} -->

| 金額 |
| --- |
| 2 |
| 3 |
| 5 <!-- xlsx-formula:"=SUM($C$4:C5)" --> |

<!-- xlsx-sheet "集計" -->

| 合計 | 文字列 |
| --- | --- |
| 5 <!-- xlsx-formula:"='売上'!C6" --> | a <!-- xlsx-formula:"=LEFT(\"a\u007cb\",1)" --> |
"#;
        let bytes = import_markdown_to_xlsx_bytes(input.as_bytes()).unwrap();
        let markdown = export_workbook_markdown(&crate::read(&bytes).unwrap()).unwrap();
        assert!(markdown.contains(r#"5 <!-- xlsx-formula:"='売上'!C6" -->"#));
        let mut rebuilt = crate::read(&import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap()).unwrap();
        let summary = sheet_of_slide(&rebuilt.slides[1]).unwrap();
        assert_eq!(summary.cell(1, 0).unwrap().formula.as_deref(), Some("'売上'!C6"));
        assert_eq!(summary.cell(1, 1).unwrap().formula.as_deref(), Some(r#"LEFT("a|b",1)"#));
        assert_eq!(summary.cell(1, 0).unwrap().value, CellValue::Number(5.0));

        let ShapeKind::Sheet(sheet) = &mut rebuilt.slides[0].shapes[0].kind else {
            panic!("expected a sheet");
        };
        let sheet = Arc::make_mut(sheet);
        let mut cells = sheet.cells.to_vec();
        cells.iter_mut().find(|cell| (cell.row, cell.column) == (3, 2)).unwrap().value = CellValue::Number(7.0);
        sheet.cells = cells.into();
        let recalculated = BookView::new(&rebuilt).recalculate(&[(0, 3, 2)]);
        for position in [(0, 5, 2), (1, 1, 0)] {
            assert!(recalculated.iter().any(|cell| cell.cell == position && cell.value == CellValue::Number(10.0)));
        }
    }

    #[test]
    fn workbook_fixed_arrays_round_trip_without_dynamic_metadata() {
        let input = r#"<!-- xlsx-sheet {"name":"配列","origin":"C3"} -->

| 1 | 3 <!-- xlsx-formula:{"formula":"=C3:C4*3","fixedArray":"D4"} --> |
| --- | --- |
| 2 | 6 |

<!-- xlsx-sheet "単一" -->

| 3 <!-- xlsx-formula:{"formula":"=SUM({1,2})","fixedArray":"A1"} --> |
| --- |
"#;
        let bytes = import_markdown_to_xlsx_bytes(input.as_bytes()).unwrap();
        let workbook = crate::read(&bytes).unwrap();
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&workbook.slides[0]).unwrap();
        assert_eq!(sheet.fixed_arrays[&(2, 3)], ibis_doc_model::CellRange {
            first_row: 2, first_column: 3, last_row: 3, last_column: 3,
        });
        assert!(sheet.spills.is_empty());
        let package = ibis_ooxml::Package::open(bytes.clone()).unwrap();
        let xml = String::from_utf8(package.read("xl/worksheets/sheet1.xml").unwrap()).unwrap();
        assert!(xml.contains(r#"t="array" ref="D3:D4""#));
        assert!(!xml.contains(r#" cm="#));
        let markdown = export_workbook_markdown(&workbook).unwrap();
        assert!(markdown.contains(r#"xlsx-formula:{"fixedArray":"D4","formula":"=C3:C4*3"}"#));
        assert!(markdown.contains(r#""fixedArray":"A1""#));
        let rebuilt = crate::read(&import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap()).unwrap();
        let rebuilt = ibis_doc_model::sheet_calc::sheet_of_slide(&rebuilt.slides[0]).unwrap();
        assert_eq!(rebuilt.fixed_arrays, sheet.fixed_arrays);
        assert_eq!(rebuilt.cell(3, 3).unwrap().value, ibis_doc_model::CellValue::Number(6.0));
        assert!(import_markdown_to_xlsx_bytes(input.replace(r#""fixedArray":"D4""#, r#""fixedArray":"C4""#).as_bytes()).is_err());
        assert!(import_markdown_to_xlsx_bytes(input.replace("| 2 | 6 |", r#"| 2 | 6 <!-- xlsx-formula:"=2*3" --> |"#).as_bytes()).is_err());
    }

    #[test]
    fn workbook_dynamic_formulas_round_trip_with_spill_ranges() {
        use ibis_doc_model::{CellError, CellRange, CellValue, sheet_calc::sheet_of_slide};

        let input = r#"<!-- xlsx-sheet {"name":"配列","origin":"C3"} -->

| 1 <!-- xlsx-formula:{"formula":"=SEQUENCE(2,2)","spill":"D4"} --> | 2 |
| --- | --- |
| 3 | 4 |
| #SPILL! <!-- xlsx-formula:{"formula":"=SEQUENCE(1,2)","spill":"C5"} --> | blocked |
"#;
        let verify = |bytes: &[u8]| {
            let workbook = crate::read(bytes).unwrap();
            let sheet = sheet_of_slide(&workbook.slides[0]).unwrap();
            assert_eq!(sheet.spills[&(2, 2)], CellRange {
                first_row: 2, first_column: 2, last_row: 3, last_column: 3,
            });
            assert_eq!(sheet.cell(2, 2).unwrap().formula.as_deref(), Some("SEQUENCE(2,2)"));
            assert_eq!(sheet.cell(3, 3).unwrap().value, CellValue::Number(4.0));
            assert_eq!(sheet.spills[&(4, 2)], CellRange {
                first_row: 4, first_column: 2, last_row: 4, last_column: 2,
            });
            assert_eq!(sheet.cell(4, 2).unwrap().value, CellValue::Error(CellError::Spill));
        };
        let original = import_markdown_to_xlsx_bytes(input.as_bytes()).unwrap();
        verify(&original);
        let markdown = export_workbook_markdown(&crate::read(&original).unwrap()).unwrap();
        assert!(markdown.contains(r#"xlsx-formula:{"formula":"=SEQUENCE(2,2)","spill":"D4"}"#));
        verify(&import_markdown_to_xlsx_bytes(markdown.as_bytes()).unwrap());

        for invalid in [input.replace("D4", "B4"), input.replace("D4", "XFE4"), input.replace("C5", "D5")] {
            assert!(open_markdown_workbook(invalid.into_bytes()).is_err());
        }
    }

    #[test]
    fn rich_footnotes_and_endnotes_round_trip_blocks() {
        let input = "本文[^1]と補足[^endnote-1]。\n\n[^1]:\n    最初の段落\n\n    次の段落\n\n    - 項目\n\n    | 項目 | 値 |\n    | --- | --- |\n    | 表の本文 | 42 |\n[^endnote-1]:\n    文末注記\n    \n    1. 手順\n";
        let original = import_markdown_to_docx_bytes(input.as_bytes()).unwrap();
        let markdown = export_docx_to_markdown(&original).unwrap();
        assert!(markdown.contains("本文[^1]と補足[^endnote-1]。"));
        assert!(markdown.contains("    | 表の本文 | 42 |"));
        let rebuilt = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        let doc = crate::read_docx(&rebuilt).unwrap();
        let flow = doc.flow.as_ref().unwrap();
        let note = &flow.footnotes[0];
        assert_eq!(note.id, 1);
        assert!(matches!(&note.blocks[..], [
            Block::Paragraph(first), Block::Paragraph(second), Block::Paragraph(item), Block::Table(_)
        ] if first.text().trim_end().ends_with("最初の段落") && second.text() == "次の段落" && item.list.is_some()),
            "{markdown}");
        assert_eq!(flow.endnotes[0].id, 1);
        assert!(matches!(&flow.endnotes[0].blocks[..], [
            Block::Paragraph(_), Block::Paragraph(item)
        ] if item.list.is_some()));
    }

    #[test]
    fn docx_first_and_even_page_parts_round_trip_with_inheritance() {
        let input = r#"---
even_and_odd_headers: true
---

First section.

::: section {"break":"next-page","title_page":true,"header":"Odd {page}","header_first":"Cover","footer_first":"","header_even":"Even {page}","footer_even":"Total {numpages}"}

Second section.

::: section {"break":"end","title_page":true,"header_first":"","footer_even":"Section {section}"}
"#;
        let first = import_markdown_to_docx_bytes(input.as_bytes()).unwrap();
        let markdown = export_docx_to_markdown(&first).unwrap();
        assert!(markdown.contains("even_and_odd_headers: true"));
        let rebuilt = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        let opened = crate::read_docx(&rebuilt).unwrap();
        let doc = opened.flow.as_ref().unwrap();
        assert!(doc.even_and_odd && doc.title_page);
        let section = doc.body.iter().find_map(|block| match block {
            Block::Paragraph(p) => p.section.as_deref(),
            _ => None,
        }).unwrap();
        assert!(section.title_page);
        let text = |part: ibis_doc_model::PartKind| {
            serialize_header_footer(section.parts[part.index()].as_deref().unwrap())
        };
        use ibis_doc_model::PartKind::*;
        assert_eq!(text(HeaderFirst), "Cover");
        assert_eq!(text(FooterFirst), "");
        assert_eq!(text(HeaderEven), "Even {page}");
        assert_eq!(text(FooterEven), "Total {numpages}");
        assert!(doc.linked[HeaderEven.index()]);
        assert!(doc.linked[FooterFirst.index()]);
        assert!(!doc.linked[HeaderFirst.index()]);
        assert_eq!(serialize_header_footer(&doc.header_first), "");
        assert_eq!(serialize_header_footer(&doc.footer_even), "Section {section}");
    }

    #[test]
    fn docx_sections_round_trip_page_geometry_break_and_headers() {
        let input = r#"縦向きの本文。

::: section {"break":"next-page","size":[11906,16838],"margins":[1440,1300,1200,1100,720,720,0],"columns":2,"column_gap":300,"column_separator":true,"header":"前半 {page}"}

横向きの本文。

::: section {"break":"end","size":[16838,11906],"columns":1,"first_page":4,"header":"後半","footer":"頁 {page}"}
"#;
        let docx = import_markdown_to_docx_bytes(input.as_bytes()).unwrap();
        let markdown = export_docx_to_markdown(&docx).unwrap();
        let rebuilt = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        let parsed = crate::read_docx(&rebuilt).unwrap();
        let flow = parsed.flow.as_ref().unwrap();
        let [Block::Paragraph(first), Block::Paragraph(last)] = flow.body.as_slice() else {
            panic!("expected the two original body paragraphs: {markdown}");
        };
        let section = first.section.as_ref().unwrap();
        assert_eq!(first.section_break, ibis_doc_model::SectionBreak::NextPage);
        assert_eq!(section.page.size, EmuSize::new(11906 * 635, 16838 * 635));
        assert_eq!(section.page.right, 1300 * 635);
        assert_eq!(section.page.columns.count, 2);
        assert_eq!(section.page.columns.space, 300 * 635);
        assert!(section.page.columns.separator);
        assert_eq!(serialize_header_footer(section.parts[0].as_deref().unwrap()), "前半 {page}");
        assert_eq!(flow.page.size, EmuSize::new(16838 * 635, 11906 * 635));
        assert!(flow.restart_numbering);
        assert_eq!(flow.page.first_page, 4);
        assert_eq!(last.text(), "横向きの本文。");
        assert_eq!(serialize_header_footer(&flow.header), "後半");
        assert_eq!(serialize_header_footer(&flow.footer), "頁 {page}");
        assert!(import_markdown_to_docx_bytes(input.replace("11906,16838", "0,16838").as_bytes()).is_err());
    }

    #[test]
    fn docx_page_fields_round_trip_in_body_and_footer() {
        let input = r#"---
footer: "{page} / {{field:pages}}"
---
# Target {#target}

Page {{field:page}}, section {{field:section}}, section pages {{field:section-pages}}, target page {{field:page-ref:target}}, formatted {{field:page:ROMAN}}.
"#;
        let docx = import_markdown_to_docx_bytes(input.as_bytes()).unwrap();
        let markdown = export_docx_to_markdown(&docx).unwrap();
        for marker in ["{{field:page}}", "{{field:pages}}", "{{field:section}}", "{{field:section-pages}}", "{{field:page-ref:target}}", "{{field:page:ROMAN}}"] {
            assert!(markdown.contains(marker), "missing {marker}: {markdown}");
        }
        let rebuilt = import_markdown_to_docx_bytes(markdown.as_bytes()).unwrap();
        let package = ibis_ooxml::Package::open(rebuilt).unwrap();
        let body = String::from_utf8(package.read("word/document.xml").unwrap()).unwrap();
        for instruction in [" PAGE ", " SECTION ", " SECTIONPAGES ", r" PAGEREF target \h ", r" PAGE \* ROMAN "] {
            assert!(body.contains(instruction), "missing {instruction}");
        }
        let footer = package.names().find(|part| part.starts_with("word/footer") && part.ends_with(".xml")).unwrap();
        let footer = String::from_utf8(package.read(footer).unwrap()).unwrap();
        assert!(footer.contains(" NUMPAGES "));
        assert_eq!(format_markdown_inline_field(r"NUMPAGES \# 000", "003"), "003");
    }

    #[test]
    fn detects_presentation_markdown_frontmatter() {
        assert!(is_presentation_markdown(b"---\nmarp: true\ntitle: Deck\n---\n\n# Slide 1\n"));
        assert!(is_presentation_markdown(b"---\r\ntype: slides\r\n---\r\n# Intro\n"));
        assert!(!is_presentation_markdown(b"---\ntitle: Notes\n---\n\n# Document\n"));
        assert!(!is_presentation_markdown(b"# Plain document\n\n---\n\nSecond section\n"));
    }

    #[test]
    fn reparse_markdown_document_preserves_view_mode_and_supports_undo() {
        let initial = "# Original\n\nHello world.\n";
        let mut opened = open_markdown(initial.as_bytes().to_vec()).expect("open_markdown");
        {
            let flow = std::sync::Arc::make_mut(opened.presentation.flow.as_mut().unwrap());
            flow.view_mode = ibis_doc_model::DocumentViewMode::Web;
        }
        let current_doc = opened.presentation.flow.as_ref().unwrap().clone();
        let mut media = opened.presentation.media.clone();
        let updated_md = "---\ntitle: Updated\n---\n\n# Revised\n\nUpdated **body**.\n";
        let new_doc = reparse_markdown_document(
            updated_md,
            &current_doc,
            &mut media,
            Some(&opened.source),
            Some(&opened.presentation),
            None,
        )
        .expect("reparse_markdown_document");
        assert_eq!(new_doc.view_mode, ibis_doc_model::DocumentViewMode::Web);

        let inverse = opened
            .presentation
            .apply(ibis_doc_model::Command::Flow(
                ibis_doc_model::FlowCommand::ReplaceDocument(Box::new(new_doc)),
            ))
            .expect("apply ReplaceDocument");
        let exported_after = export_markdown(
            opened.presentation.flow.as_ref().unwrap(),
            &opened.presentation.media,
        );
        assert!(exported_after.contains("# Revised"));
        assert!(exported_after.contains("Updated **body**."));

        opened.presentation.apply(inverse).expect("undo ReplaceDocument");
        let exported_undone = export_markdown(
            opened.presentation.flow.as_ref().unwrap(),
            &opened.presentation.media,
        );
        assert!(exported_undone.contains("# Original"));
        assert!(exported_undone.contains("Hello world."));
    }
}
