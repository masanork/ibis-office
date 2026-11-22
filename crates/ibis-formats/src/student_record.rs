// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Read candidate values from the supported elementary school record table.

use ibis_doc_model::flow::{Block, FlowCell, FlowDocument, FlowRunKind, FlowTable, VerticalMerge};

#[derive(Debug, PartialEq, Eq)]
pub struct StudentRecord {
    pub name: Candidate,
    pub birth_date: Candidate,
    pub next_school: Candidate,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Candidate {
    pub text: String,
    pub paragraph: u32,
}

fn plain_text(blocks: &[Block]) -> Option<String> {
    let mut text = String::new();
    for (index, block) in blocks.iter().enumerate() {
        let Block::Paragraph(paragraph) = block else {
            return None;
        };
        if index > 0 {
            text.push('\n');
        }
        for run in &paragraph.runs {
            if run.revision.is_some() {
                return None;
            }
            match &run.kind {
                FlowRunKind::Text(value) => text.push_str(value),
                FlowRunKind::Ruby { base, .. } => text.push_str(base),
                FlowRunKind::LineBreak => text.push('\n'),
                FlowRunKind::Tab | FlowRunKind::PositionalTab { .. } => text.push('\t'),
                _ => return None,
            }
        }
    }
    Some(text.trim().to_owned())
}

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

struct Cell<'a> {
    row: usize,
    column: u64,
    grid_end: u64,
    source: &'a FlowCell,
    label: String,
}

fn table_record(table: &FlowTable) -> Option<StudentRecord> {
    let mut cells = Vec::new();
    for (row, line) in table.rows.iter().enumerate() {
        let mut column = 0;
        for source in &line.cells {
            let grid_end = column + u64::from(source.span.max(1));
            let label = plain_text(&source.blocks).map_or_else(String::new, |text| compact(&text));
            cells.push(Cell {
                row,
                column,
                grid_end,
                source,
                label,
            });
            column = grid_end;
        }
        if column != table.columns.len() as u64 {
            return None;
        }
    }
    let unique = |label: &str| {
        let mut matching = cells
            .iter()
            .filter(|cell| cell.label == label && cell.source.merge != VerticalMerge::Continue);
        let result = matching.next()?;
        matching.next().is_none().then_some(result)
    };
    let heading = unique("学籍の記録")?;
    let section = unique("児童")?;
    let guardian = unique("保護者")?;
    if heading.row >= section.row
        || section.row >= guardian.row
        || section.column != guardian.column
    {
        return None;
    }

    // Stop at the first row outside the vertically merged pupil label.
    let mut end = section.row + 1;
    if section.source.merge == VerticalMerge::Start {
        for row in end..guardian.row {
            if !cells.iter().any(|cell| {
                cell.row == row
                    && cell.column == section.column
                    && cell.grid_end == section.grid_end
                    && cell.source.merge == VerticalMerge::Continue
            }) {
                break;
            }
            end += 1;
        }
    }
    let student_label = |label: &str| {
        let mut matching = cells.iter().filter(|cell| {
            (section.row..end).contains(&cell.row)
                && cell.column == section.grid_end
                && cell.label == label
                && cell.source.merge == VerticalMerge::None
        });
        let result = matching.next()?;
        matching.next().is_none().then_some(result)
    };
    let value = |label: &Cell<'_>, limit| {
        let cell = cells
            .iter()
            .find(|cell| cell.row == label.row && cell.column == label.grid_end)?;
        if cell.source.merge == VerticalMerge::Continue || cell.grid_end > limit {
            return None;
        }
        let Block::Paragraph(paragraph) = cell.source.blocks.first()? else {
            return None;
        };
        Some(Candidate {
            text: plain_text(&cell.source.blocks)?,
            paragraph: paragraph.id,
        })
    };
    let name_label = student_label("氏名")?;
    let date_label = student_label("生年月日")?;
    let school_label = unique("進学先")?;
    if date_label.grid_end != name_label.grid_end
        || school_label.column <= date_label.grid_end
        || school_label.row < end
    {
        return None;
    }
    let mut birth_date = value(date_label, school_label.column)?;
    if matches!(compact(&birth_date.text).as_str(), "年月日" | "年月日生") {
        birth_date.text.clear();
    }
    Some(StudentRecord {
        name: value(name_label, school_label.column)?,
        birth_date,
        next_school: value(school_label, u64::MAX)?,
    })
}

/// Recognize a single supported form. Blank cells are valid candidates.
pub fn extract(document: &FlowDocument) -> Option<StudentRecord> {
    let mut forms = document.body.iter().filter_map(|block| match block {
        Block::Table(table) => table_record(table),
        Block::Paragraph(_) => None,
    });
    let form = forms.next()?;
    forms.next().is_none().then_some(form)
}
