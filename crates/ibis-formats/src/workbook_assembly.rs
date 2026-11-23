// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use crate::{Error, Kind, Opened};
use ibis_doc_model::ShapeKind;
use ibis_doc_model::sheet::Sheet;
use ibis_doc_model::sheet_calc::sheet_of_slide;
use ibis_doc_model::sheet_ops::{StructureEdit, apply_structure};
use std::sync::Arc;

/// Assemble a new workbook using prepared worksheets in display order.
pub(crate) fn assemble(
    prepare: impl FnOnce(&Sheet) -> Result<Vec<(String, Sheet)>, Error>,
) -> Result<Opened, Error> {
    let mut opened = crate::open(crate::blank_file(Kind::Workbook)?)?;
    let book = &mut opened.presentation;
    let template = book
        .slides
        .first()
        .and_then(sheet_of_slide)
        .ok_or_else(|| Error::new("Missing blank worksheet"))?;
    let sheets = prepare(template)?;

    for (index, (name, sheet)) in sheets.into_iter().enumerate() {
        let content = Arc::new(sheet);
        let edit = if index == 0 {
            let shape = book.slides[0]
                .shapes
                .iter_mut()
                .find(|shape| matches!(shape.kind, ShapeKind::Sheet(_)))
                .ok_or_else(|| Error::new("Missing blank worksheet"))?;
            shape.kind = ShapeKind::Sheet(content);
            StructureEdit::RenameSheet { sheet: 0, name }
        } else {
            StructureEdit::AddSheet {
                sheet: index,
                name,
                slide: book.new_slide_id(),
                shape: book.new_shape_id(),
                content,
            }
        };
        apply_structure(book, &edit);
        Arc::make_mut(
            book.workbook
                .as_mut()
                .ok_or_else(|| Error::new("Missing workbook"))?,
        )
        .structure
        .push(edit);
    }
    Ok(opened)
}
