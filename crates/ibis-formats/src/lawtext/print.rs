// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::sync::Arc;

use ibis_doc_model::flow::{
    Block, BorderStyle, FlowBorder, FlowTable, UnderlineStyle, format_list_number,
};
use ibis_doc_model::{FlowAlign, Rgb};

use super::Legislation5SetPackage;
use crate::{Error, SaveOptions, markdown};

const COMPARISON_BOOKMARK: &str = "lawtext_comparison";

const PAGE_STYLE: &str = "\
---
size: A4
direction: vertical
margins.top: 24mm
margins.bottom: 24mm
margins.left: 22mm
margins.right: 22mm
font: Noto Serif CJK JP
font_size: 11pt
paragraph_spacing: compact
footer: '- {page} -'
---

";

impl Legislation5SetPackage {
    /// Produce a single editable, A4 vertical-writing DOCX of the five drafts.
    pub fn to_print_docx(&self) -> Result<Vec<u8>, Error> {
        let bill = self.bill_markdown.replace(
            &self.aratame_draft.full_text,
            &self
                .aratame_draft
                .full_text
                .lines()
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
        // Keep the comparison markup and contents from the common diff pipeline.
        let comparison = self
            .shinkyu_markdown
            .replacen(
                " 新旧対照表\n",
                &format!(" 新旧対照条文 {{#{COMPARISON_BOOKMARK}}}\n"),
                1,
            )
            .replacen("\n| 改正後 |", "\n| 改正案 |", 1)
            .replacen("| 改正後 |\n", "| 改正案 |\n", 1);
        let outline = numbered_outline(&self.youkou_markdown);
        let document = [
            outline.trim_end(),
            bill.trim_end(),
            self.riyuu_markdown.trim_end(),
            comparison.trim_end(),
            self.sansho_markdown.trim_end(),
        ]
        .join("\n\n\\newpage\n\n");

        let mut opened = markdown::open_markdown(format!("{PAGE_STYLE}{document}").into_bytes())?;
        let flow = Arc::make_mut(opened.presentation.flow.as_mut().unwrap());
        // Place the running number below the table's lower rule.
        flow.page.footer = 12 * 36_000;
        let available = flow.page.size.height - flow.page.top - flow.page.bottom;
        let table = flow
            .body
            .iter_mut()
            .skip_while(|block| {
                !matches!(block, Block::Paragraph(p)
                if p.bookmarks.iter().any(|name| name == COMPARISON_BOOKMARK))
            })
            .find_map(|block| match block {
                Block::Table(table) => Some(table),
                _ => None,
            })
            .ok_or_else(|| Error::new("新旧対照条文の表を組版できません"))?;
        format_comparison(table, available);
        opened
            .source
            .save(&opened.presentation, SaveOptions::default())
    }
}

fn numbered_outline(outline: &str) -> String {
    let mut article = 0;
    outline
        .lines()
        .map(|line| {
            if let Some(text) = line.strip_prefix("- ") {
                article += 1;
                format!(
                    "{}　{text}\n\n",
                    format_list_number(article, "japaneseCounting")
                )
            } else if let Some(text) = line.strip_prefix("  - ") {
                format!("　{text}\n\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect()
}

fn format_comparison(table: &mut FlowTable, available: ibis_doc_model::Emu) {
    // In a vertical section, the logical table columns occupy the upper and
    // lower bands of the paper. Let Word and the PDF renderer split long rows.
    let width = available / 2 / 635 * 635;
    table.columns = vec![width; 2];
    table.style = None;
    let rule = FlowBorder {
        width: 6_350,
        color: Rgb(0),
        style: BorderStyle::Single,
        space: 0,
        art: None,
    };
    table.borders = [Some(Some(rule)); 6];
    table.borders[4] = Some(None);
    for (index, row) in table.rows.iter_mut().enumerate() {
        for cell in &mut row.cells {
            cell.shading = None;
            cell.own_borders = [None; 4];
            if index <= 1 {
                // A single rule separates the repeated labels from the text.
                cell.own_borders[if index == 0 { 2 } else { 0 }] = Some(Some(rule));
            }
            for block in &mut cell.blocks {
                let Block::Paragraph(paragraph) = block else {
                    continue;
                };
                paragraph.table_style = None;
                if index == 0 {
                    paragraph.direct.align = Some(FlowAlign::Center);
                }
                for run in &mut paragraph.runs {
                    if run.style.strike {
                        // The review Markdown strikes deletions. The printed
                        // comparison marks the changed text on both sides.
                        run.format.strike = None;
                        run.format.underline = Some(true);
                        run.format.underline_style = Some(UnderlineStyle::Single);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lawtext::{ShinkyuOptions, build_5set_package, parse_law_input};
    use ibis_doc_model::flow::FlowRunKind;

    #[test]
    fn print_docx_preserves_page_breaks_rules_and_change_marks() {
        let old = parse_law_input("試験法（令和七年法律第一号）\n第一条　赤色を用いる。").unwrap();
        let new = parse_law_input("試験法（令和七年法律第一号）\n第一条　青色を用いる。").unwrap();
        let set = build_5set_package(&old, &new, "", &ShinkyuOptions::default());
        let bytes = set.to_print_docx().unwrap();
        let opened = crate::open(bytes).unwrap();
        let flow = opened.presentation.flow.unwrap();
        assert_eq!(flow.page.direction, ibis_doc_model::CellDirection::TbRl);
        assert!(flow.page.footer < flow.page.bottom);
        assert!(flow.body.iter().any(|block| matches!(block, Block::Paragraph(p)
            if p.runs.iter().any(|run| matches!(&run.kind, FlowRunKind::Text(t) if t.starts_with("一　第一条"))))));
        let breaks = flow
            .body
            .iter()
            .filter(|block| {
                matches!(block, Block::Paragraph(paragraph) if paragraph.runs.iter().any(|run|
                matches!(run.kind, FlowRunKind::PageBreak)))
            })
            .count();
        assert_eq!(breaks, 4);
        let table = flow
            .body
            .iter()
            .find_map(|block| {
                if let Block::Table(table) = block {
                    Some(table)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(table.columns.len(), 2);
        assert!(
            (table.columns.iter().sum::<ibis_doc_model::Emu>()
                - (flow.page.size.height - flow.page.top - flow.page.bottom))
                .abs()
                <= 1_270
        );
        assert!(table.rows[0].header);
        assert_eq!(table.borders[5].flatten().unwrap().color, Rgb(0));
        for cell in &table.rows[1].cells {
            let Block::Paragraph(p) = &cell.blocks[0] else {
                panic!("missing comparison text")
            };
            assert!(p.runs.iter().any(|run| run.style.underline));
            assert!(p.runs.iter().all(|run| !run.style.strike));
        }
        build_5set_package(&old, &old, "", &ShinkyuOptions::default())
            .to_print_docx()
            .unwrap();
    }
}
