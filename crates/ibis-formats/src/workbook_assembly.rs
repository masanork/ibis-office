// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use crate::{Error, Kind, Opened};
use ibis_doc_model::sheet::Sheet;
use ibis_doc_model::sheet_calc::sheet_of_slide;
use ibis_doc_model::sheet_ops::{StructureEdit, apply_structure};
use ibis_doc_model::{Presentation, ShapeKind};
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
        install_sheet(book, index, Some(name), sheet)?;
    }
    Ok(opened)
}

/// Install the first prepared sheet into the blank tab, or append the next named sheet.
pub(crate) fn install_sheet(
    book: &mut Presentation,
    index: usize,
    name: Option<String>,
    sheet: Sheet,
) -> Result<(), Error> {
    if index == 0 {
        // The prepared content already uses the final sheet names.
        if let Some(name) = name {
            record_structure(book, StructureEdit::RenameSheet { sheet: 0, name })?;
        }
        let shape = book
            .slides
            .first_mut()
            .and_then(|slide| {
                slide
                    .shapes
                    .iter_mut()
                    .find(|shape| matches!(shape.kind, ShapeKind::Sheet(_)))
            })
            .ok_or_else(|| Error::new("Missing blank worksheet"))?;
        shape.kind = ShapeKind::Sheet(Arc::new(sheet));
        return Ok(());
    }
    let edit = StructureEdit::AddSheet {
        sheet: index,
        name: name.ok_or_else(|| Error::new("Missing worksheet name"))?,
        slide: book.new_slide_id(),
        shape: book.new_shape_id(),
        content: Arc::new(sheet),
    };
    record_structure(book, edit)
}

fn record_structure(book: &mut Presentation, edit: StructureEdit) -> Result<(), Error> {
    apply_structure(book, &edit);
    Arc::make_mut(
        book.workbook
            .as_mut()
            .ok_or_else(|| Error::new("Missing workbook"))?,
    )
    .structure
    .push(edit);
    Ok(())
}
