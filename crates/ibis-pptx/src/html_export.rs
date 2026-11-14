// SPDX-License-Identifier: AGPL-3.0-only
//! Standalone, structured HTML for visible workbook sheets and presentation slides.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::{Error, docx_convert::xml_escape};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ibis_doc_model::{
    CellAlign, CellKind, Fill, MediaFormat, Presentation, Shape, ShapeKind, Sheet, SheetState,
    TextBody,
};

// Bound the rendered grid before allocating or looping over sparse worksheets.
const MAX_GRID_CELLS: u64 = 250_000;

fn start_html(title: &str, css: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"UTF-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>{css}</style></head><body>",
        xml_escape(title)
    )
}
fn row_visible(s: &Sheet, r: u32) -> bool {
    !s.hidden_rows.contains_key(&r)
        && !s.filtered_rows.contains(&r)
        && s.rows.get(r as usize).is_none_or(|v| *v != 0)
}
fn col_visible(s: &Sheet, c: u32) -> bool {
    !s.hidden_columns.contains_key(&c) && s.columns.get(c as usize).is_none_or(|v| *v != 0)
}

/// Exports the displayed values of every visible worksheet. Hidden sheets,
/// rows and columns are omitted. Excessively sparse grids return an error.
pub fn export_workbook_html(p: &Presentation, title: &str) -> Result<Vec<u8>, Error> {
    let book = p
        .workbook
        .as_ref()
        .ok_or_else(|| Error::new("Workbook data is unavailable"))?;
    let mut out = start_html(
        title,
        "body{font:14px system-ui,sans-serif;margin:1.5em}section{margin-bottom:2em}.sheet{overflow:auto}table{border-collapse:collapse}td,th{border:1px solid #aaa;padding:.3em .5em;min-width:3em;white-space:pre-wrap}th{background:#eee}",
    );
    let mut total = 0u64;
    for sheet_ref in &book.sheets {
        if sheet_ref.state != SheetState::Visible || sheet_ref.other {
            continue;
        }
        let Some(sheet) = sheet_ref
            .slide
            .and_then(|id| p.slide(id))
            .and_then(ibis_doc_model::sheet_calc::sheet_of_slide)
        else {
            return Err(Error::new("A visible worksheet could not be exported"));
        };
        if sheet
            .cells
            .iter()
            .any(|cell| cell.row >= 1_048_576 || cell.column >= 16_384)
            || sheet.merges.iter().any(|m| {
                m.first_row > m.last_row
                    || m.first_column > m.last_column
                    || m.last_row >= 1_048_576
                    || m.last_column >= 16_384
            })
        {
            return Err(Error::new("The workbook contains an invalid cell range"));
        }
        write!(
            out,
            "<section><h2>{}</h2><div class=\"sheet\"><table>",
            xml_escape(&sheet_ref.name)
        )
        .unwrap();
        let mut occupied = sheet
            .cells
            .iter()
            .filter(|cell| row_visible(sheet, cell.row) && col_visible(sheet, cell.column));
        if let Some(first) = occupied.next() {
            let (mut minr, mut maxr, mut minc, mut maxc) =
                (first.row, first.row, first.column, first.column);
            for c in occupied {
                minr = minr.min(c.row);
                maxr = maxr.max(c.row);
                minc = minc.min(c.column);
                maxc = maxc.max(c.column);
            }
            // Include a merged region only if its anchor is visible and present.
            for m in &sheet.merges {
                if row_visible(sheet, m.first_row)
                    && col_visible(sheet, m.first_column)
                    && sheet.cell(m.first_row, m.first_column).is_some()
                {
                    minr = minr.min(m.first_row);
                    maxr = maxr.max(m.last_row);
                    minc = minc.min(m.first_column);
                    maxc = maxc.max(m.last_column);
                }
            }
            let area = (u64::from(maxr) - u64::from(minr) + 1)
                .saturating_mul(u64::from(maxc) - u64::from(minc) + 1);
            total = total.saturating_add(area);
            if total > MAX_GRID_CELLS {
                return Err(Error::new(
                    "The workbook is too large or sparse to export as HTML",
                ));
            }
            let rows: Vec<_> = (minr..=maxr).filter(|&r| row_visible(sheet, r)).collect();
            let cols: Vec<_> = (minc..=maxc).filter(|&c| col_visible(sheet, c)).collect();
            let mut covered = BTreeSet::new();
            let mut spans = BTreeMap::new();
            for m in &sheet.merges {
                if !row_visible(sheet, m.first_row)
                    || !col_visible(sheet, m.first_column)
                    || sheet.cell(m.first_row, m.first_column).is_none()
                {
                    continue;
                }
                let mr: Vec<_> = rows
                    .iter()
                    .copied()
                    .filter(|r| *r >= m.first_row && *r <= m.last_row)
                    .collect();
                let mc: Vec<_> = cols
                    .iter()
                    .copied()
                    .filter(|c| *c >= m.first_column && *c <= m.last_column)
                    .collect();
                if mr.is_empty() || mc.is_empty() {
                    continue;
                }
                spans.insert((mr[0], mc[0]), (mr.len(), mc.len()));
                for &r in &mr {
                    for &c in &mc {
                        if (r, c) != (mr[0], mc[0]) {
                            covered.insert((r, c));
                        }
                    }
                }
            }
            out.push_str("<thead><tr><th></th>");
            for &c in &cols {
                write!(
                    out,
                    "<th scope=\"col\">{}</th>",
                    ibis_doc_model::sheet::cell_name(c, 0).trim_end_matches('1')
                )
                .unwrap();
            }
            out.push_str("</tr></thead><tbody>");
            for &r in &rows {
                write!(out, "<tr><th scope=\"row\">{}</th>", r + 1).unwrap();
                for &c in &cols {
                    if covered.contains(&(r, c)) {
                        continue;
                    }
                    let cell = sheet.cell(r, c);
                    write!(
                        out,
                        "<td data-cell=\"{}\"",
                        ibis_doc_model::sheet::cell_name(c, r)
                    )
                    .unwrap();
                    if let Some(&(rs, cs)) = spans.get(&(r, c)) {
                        if rs > 1 {
                            write!(out, " rowspan=\"{rs}\"").unwrap();
                        }
                        if cs > 1 {
                            write!(out, " colspan=\"{cs}\"").unwrap();
                        }
                    }
                    if let Some(cell) = cell {
                        if let Some(st) = sheet.styles.get(cell.style as usize) {
                            let align = match st.align {
                                CellAlign::Left => "left",
                                CellAlign::Center | CellAlign::CenterAcross => "center",
                                CellAlign::Right => "right",
                                _ => match cell.kind {
                                    CellKind::Number => "right",
                                    CellKind::Boolean | CellKind::Error => "center",
                                    _ => "left",
                                },
                            };
                            write!(
                                out,
                                " style=\"text-align:{align};color:#{:06X};",
                                st.color.0 & 0xffffff
                            )
                            .unwrap();
                            if let Some(color) = st.fill {
                                write!(out, "background-color:#{:06X};", color.0 & 0xffffff)
                                    .unwrap();
                            }
                            if st.bold {
                                out.push_str("font-weight:bold;");
                            }
                            if st.italic {
                                out.push_str("font-style:italic;");
                            }
                            if st.underline.enabled() {
                                out.push_str("text-decoration:underline;");
                            }
                            out.push('"');
                        }
                        write!(out, ">{}</td>", xml_escape(&cell.text)).unwrap();
                    } else {
                        out.push_str("></td>");
                    }
                }
                out.push_str("</tr>");
            }
            out.push_str("</tbody>");
        }
        out.push_str("</table></div></section>");
    }
    out.push_str("</body></html>");
    Ok(out.into_bytes())
}

fn body_html(out: &mut String, body: &TextBody, slide_width: i64) {
    for para in &body.paragraphs {
        out.push_str("<p>");
        if let Some(bullet) = &para.bullet {
            write!(out, "{} ", xml_escape(&bullet.text)).unwrap();
        }
        for run in &para.runs {
            if run.style.invisible {
                continue;
            }
            let size = run.style.font_size.unwrap_or(body.style.font_size) as f64 * 127.0 * 100.0
                / slide_width.max(1) as f64;
            write!(out, "<span style=\"font-size:{size:.4}cqw;").unwrap();
            if run.style.bold.unwrap_or(body.style.bold) {
                out.push_str("font-weight:bold;");
            }
            if run.style.italic.unwrap_or(body.style.italic) {
                out.push_str("font-style:italic;");
            }
            if let Some(color) = run.style.color {
                write!(out, "color:#{:06X};", color.0 & 0xffffff).unwrap();
            }
            if run.style.underline {
                out.push_str("text-decoration:underline;");
            }
            write!(out, "\">{}</span>", xml_escape(&run.text)).unwrap();
        }
        out.push_str("</p>");
    }
}
fn shape_html(out: &mut String, p: &Presentation, shape: &Shape, width: i64, height: i64) {
    let f = shape.frame;
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let mut content = String::new();
    if let Some(body) = shape.text_body() {
        body_html(&mut content, body, width);
    }
    match &shape.kind {
        ShapeKind::Picture(pic) => {
            if let Some(media) = p.media.iter().find(|m| m.id == pic.media) {
                let mime = match media.format {
                    MediaFormat::Png => Some("image/png"),
                    MediaFormat::Jpeg => Some("image/jpeg"),
                    MediaFormat::Gif => Some("image/gif"),
                    MediaFormat::Webp => Some("image/webp"),
                    _ => None,
                };
                if let Some(mime) = mime {
                    write!(
                        content,
                        "<img alt=\"{}\" src=\"data:{mime};base64,{}\">",
                        xml_escape(&pic.alt_text.description),
                        STANDARD.encode(&media.data)
                    )
                    .unwrap();
                }
            }
        }
        ShapeKind::Table(t) => {
            content.push_str("<table>");
            for row in &t.rows {
                content.push_str("<tr>");
                for cell in &row.cells {
                    if cell.merged {
                        continue;
                    }
                    write!(
                        content,
                        "<td rowspan=\"{}\" colspan=\"{}\"",
                        cell.row_span.max(1),
                        cell.column_span.max(1)
                    )
                    .unwrap();
                    if let Fill::Solid(color) = cell.fill {
                        write!(
                            content,
                            " style=\"background-color:#{:06X}\"",
                            color.rgb.0 & 0xffffff
                        )
                        .unwrap();
                    }
                    content.push('>');
                    body_html(&mut content, &cell.text, width);
                    content.push_str("</td>");
                }
                content.push_str("</tr>");
            }
            content.push_str("</table>");
        }
        _ => {}
    }
    let visible_auto = matches!(&shape.kind, ShapeKind::Auto(auto) if !matches!(auto.fill, Fill::None) || auto.stroke.is_some());
    if content.is_empty() && !visible_auto {
        return;
    }
    write!(out,"<div class=\"shape\" style=\"left:{:.5}%;top:{:.5}%;width:{:.5}%;height:{:.5}%;transform:rotate({:.4}deg);",f.x as f64*100./w,f.y as f64*100./h,f.width.max(0) as f64*100./w,f.height.max(0) as f64*100./h,shape.rotation as f64/60000.).unwrap();
    if let ShapeKind::Auto(auto) = &shape.kind {
        if let Fill::Solid(color) = auto.fill {
            write!(out, "background-color:#{:06X};", color.rgb.0 & 0xffffff).unwrap();
        }
        if let Some(stroke) = &auto.stroke {
            write!(
                out,
                "border:{:.4}cqw solid #{:06X};box-sizing:border-box;",
                stroke.width.max(0) as f64 * 100.0 / w,
                stroke.color.rgb.0 & 0xffffff
            )
            .unwrap();
        }
        match auto.geometry {
            ibis_doc_model::Geometry::Ellipse => out.push_str("border-radius:50%;"),
            ibis_doc_model::Geometry::RoundRect { .. } => out.push_str("border-radius:12%;"),
            _ => {}
        }
    }
    write!(out, "\">{content}</div>").unwrap();
}

/// Exports visible slides with positioned text, basic tables and raster images.
/// Speaker notes and hidden slides/shapes are not included.
pub fn export_slides_html(p: &Presentation, title: &str) -> Result<Vec<u8>, Error> {
    if p.flow.is_some() || p.workbook.is_some() {
        return Err(Error::new("Presentation data is unavailable"));
    }
    let mut out = start_html(
        title,
        "body{font:14px system-ui,sans-serif;margin:1em;background:#eee}.slide{width:min(100%,1100px);margin:1em auto;background:white;position:relative;overflow:hidden;container-type:inline-size}.shape{position:absolute;overflow:hidden;white-space:pre-wrap}.shape p{margin:0}.shape img{width:100%;height:100%;object-fit:fill}.shape table{width:100%;height:100%;border-collapse:collapse}.shape td{border:1px solid #888}",
    );
    for (i, slide) in p.slides.iter().enumerate() {
        if slide.hidden {
            continue;
        }
        let size = slide.size.unwrap_or(p.slide_size);
        let (w, h) = (size.width.max(1), size.height.max(1));
        write!(
            out,
            "<section aria-label=\"Slide {}\" class=\"slide\" style=\"aspect-ratio:{w}/{h};",
            i + 1
        )
        .unwrap();
        if let Fill::Solid(color) = slide.background {
            write!(out, "background-color:#{:06X};", color.rgb.0 & 0xffffff).unwrap();
        }
        out.push_str("\">");
        if !slide.hide_backdrop {
            for shape in &slide.backdrop {
                shape_html(&mut out, p, shape, w, h);
            }
        }
        for shape in &slide.shapes {
            if !slide.hidden_shapes.contains(&shape.id) {
                shape_html(&mut out, p, shape, w, h);
            }
        }
        out.push_str("</section>");
    }
    out.push_str("</body></html>");
    Ok(out.into_bytes())
}
