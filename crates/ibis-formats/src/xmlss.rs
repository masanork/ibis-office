// SPDX-License-Identifier: AGPL-3.0-only
//! Import the standalone Excel XML Spreadsheet 2003 format as an editable workbook.
use std::{borrow::Cow, collections::HashSet, fmt::Write};

use roxmltree::{Document, Node};

use crate::{Error, Opened, csv::column_name, docx_convert::xml_escape as esc};

const SS: &str = "urn:schemas-microsoft-com:office:spreadsheet";
const EXCEL: &str = "urn:schemas-microsoft-com:office:excel";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const ROWS: u32 = 1_048_576;
const COLUMNS: u32 = 16_384;

fn invalid(reason: &str) -> Error {
    Error::new(format!("Excel XML Spreadsheet 2003: {reason}"))
}

fn xml_text(bytes: &[u8]) -> Option<Cow<'_, str>> {
    if matches!(bytes.get(..2), Some([0xff, 0xfe] | [0xfe, 0xff])) {
        let (encoding, _) = encoding_rs::Encoding::for_bom(bytes)?;
        return encoding.decode_without_bom_handling_and_without_replacement(&bytes[2..]);
    }
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).ok()?;
    text.trim_start()
        .starts_with('<')
        .then_some(Cow::Borrowed(text))
}

pub fn try_open(data: &[u8]) -> Result<Option<Opened>, Error> {
    let Some(text) = xml_text(data).filter(|s| s.contains(SS)) else {
        return Ok(None);
    };
    let Ok(document) = Document::parse(&text) else {
        return Ok(None);
    };
    let workbook = document.root_element();
    if !workbook.has_tag_name((SS, "Workbook")) {
        return Ok(None);
    }
    crate::open(to_xlsx(workbook)?).map(Some)
}

fn children<'a, 'input>(
    node: Node<'a, 'input>,
    tag: &'static str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    node.children()
        .filter(move |child| child.has_tag_name((SS, tag)))
}

fn integer(node: Node<'_, '_>, key: &str, default: u32) -> Result<u32, Error> {
    node.attribute((SS, key)).map_or(Ok(default), |v| {
        v.parse().map_err(|_| invalid(&format!("invalid {key}")))
    })
}

fn position(node: Node<'_, '_>, next: u32, max: u32) -> Result<u32, Error> {
    let index = integer(node, "Index", next)?;
    if index < next || index > max {
        return Err(invalid(
            "cell position is outside the sheet or out of order",
        ));
    }
    Ok(index)
}

fn address(row: u32, column: u32) -> String {
    format!("{}{row}", column_name(column - 1))
}

fn sheet_xml(sheet: Node<'_, '_>, date1904: bool) -> Result<String, Error> {
    let mut xml = format!(r#"<worksheet xmlns="{MAIN}"><sheetData>"#);
    let mut merges = String::new();
    if let Some(table) = children(sheet, "Table").next() {
        let top = integer(table, "TopCell", 1)?
            .checked_sub(1)
            .ok_or_else(|| invalid("invalid TopCell"))?;
        let left = integer(table, "LeftCell", 1)?
            .checked_sub(1)
            .ok_or_else(|| invalid("invalid LeftCell"))?;
        let mut next_row = 1;
        for source in children(table, "Row") {
            let index = position(source, next_row, ROWS.saturating_sub(top))?;
            let row = index + top;
            let span = integer(source, "Span", 0)?;
            next_row = index
                .checked_add(span)
                .filter(|r| *r <= ROWS - top)
                .ok_or_else(|| invalid("row span exceeds the sheet"))?
                + 1;
            write!(xml, r#"<row r="{row}">"#).unwrap();
            let mut next_col = 1;
            for cell in children(source, "Cell") {
                let index = position(cell, next_col, COLUMNS.saturating_sub(left))?;
                let column = index + left;
                let right = column
                    .checked_add(integer(cell, "MergeAcross", 0)?)
                    .filter(|v| *v <= COLUMNS)
                    .ok_or_else(|| invalid("column merge exceeds the sheet"))?;
                let bottom = row
                    .checked_add(integer(cell, "MergeDown", 0)?)
                    .filter(|v| *v <= ROWS)
                    .ok_or_else(|| invalid("row merge exceeds the sheet"))?;
                next_col = right - left + 1;
                let at = address(row, column);
                if bottom != row || right != column {
                    write!(
                        merges,
                        r#"<mergeCell ref="{at}:{}"/>"#,
                        address(bottom, right)
                    )
                    .unwrap();
                }
                cell_xml(&mut xml, cell, &at, row, column, date1904)?;
            }
            xml.push_str("</row>");
        }
    }
    xml.push_str("</sheetData>");
    if !merges.is_empty() {
        write!(xml, "<mergeCells>{merges}</mergeCells>").unwrap();
    }
    xml.push_str("</worksheet>");
    Ok(xml)
}

fn cell_xml(
    xml: &mut String,
    cell: Node<'_, '_>,
    at: &str,
    row: u32,
    column: u32,
    date1904: bool,
) -> Result<(), Error> {
    let formula = cell
        .attribute((SS, "Formula"))
        .and_then(|s| s.strip_prefix('='))
        .filter(|_| cell.attribute((SS, "ArrayRange")).is_none())
        .map(|s| formula_to_a1(s, row, column));
    let Some(data) = children(cell, "Data").next() else {
        if let Some(f) = formula {
            write!(xml, r#"<c r="{at}"><f>{}</f></c>"#, esc(&f)).unwrap();
        }
        return Ok(());
    };
    let text: String = data
        .descendants()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
        .collect();
    let trimmed = text.trim();
    let (kind, style, value) = match data.attribute((SS, "Type")).unwrap_or("String") {
        "String" if formula.is_none() => {
            write!(
                xml,
                r#"<c r="{at}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                esc(&text)
            )
            .unwrap();
            return Ok(());
        }
        "String" => ("str", 0, text),
        "Number" => (
            "n",
            0,
            trimmed
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .ok_or_else(|| invalid("invalid numeric cell"))?
                .to_string(),
        ),
        "Boolean" => (
            "b",
            0,
            match trimmed {
                "0" | "false" => "0",
                "1" | "true" => "1",
                _ => return Err(invalid("invalid boolean cell")),
            }
            .to_owned(),
        ),
        "Error" => ("e", 0, trimmed.to_owned()),
        "DateTime" => {
            let iso = trimmed.replace('T', " ");
            let serial = ibis_calc::eval::convert::parse_date_time(&iso, date1904)
                .ok_or_else(|| invalid("invalid date cell"))?;
            ("n", 1, serial.to_string())
        }
        _ => return Err(invalid("unsupported cell data type")),
    };
    write!(xml, r#"<c r="{at}" t="{kind}" s="{style}">"#).unwrap();
    if let Some(f) = formula {
        write!(xml, "<f>{}</f>", esc(&f)).unwrap();
    }
    write!(xml, "<v>{}</v></c>", esc(&value)).unwrap();
    Ok(())
}

fn to_xlsx(workbook: Node<'_, '_>) -> Result<Vec<u8>, Error> {
    let mut zip = ibis_ooxml::zip::Writer::new();
    let mut add = |path: &str, text: &str| {
        zip.add(path, Default::default(), text.as_bytes())
            .map_err(|e| invalid(&e.to_string()))
    };
    let date1904 = workbook
        .children()
        .find(|n| n.has_tag_name((EXCEL, "ExcelWorkbook")))
        .is_some_and(|n| n.children().any(|n| n.has_tag_name((EXCEL, "Date1904"))));
    let mut rels = format!(
        r#"<Relationships xmlns="{PACKAGE_REL}"><Relationship Id="styles" Type="{REL}/styles" Target="styles.xml"/>"#
    );
    let mut types = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    let mut sheets = String::new();
    let mut names = HashSet::new();
    for (index, worksheet) in children(workbook, "Worksheet").enumerate() {
        let name = worksheet
            .attribute((SS, "Name"))
            .ok_or_else(|| invalid("worksheet has no name"))?;
        if name.is_empty()
            || name.encode_utf16().count() > 31
            || name.contains(['[', ']', ':', '*', '?', '/', '\\'])
            || !names.insert(name.to_lowercase())
        {
            return Err(invalid("invalid or duplicate worksheet name"));
        }
        let id = index + 1;
        let path = format!("worksheets/sheet{id}.xml");
        add(&format!("xl/{path}"), &sheet_xml(worksheet, date1904)?)?;
        write!(
            sheets,
            r#"<sheet name="{}" sheetId="{id}" r:id="s{id}"/>"#,
            esc(name)
        )
        .unwrap();
        write!(
            rels,
            r#"<Relationship Id="s{id}" Type="{REL}/worksheet" Target="{path}"/>"#
        )
        .unwrap();
        write!(types, r#"<Override PartName="/xl/{path}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#).unwrap();
    }
    if sheets.is_empty() {
        return Err(invalid("no worksheets"));
    }
    add("[Content_Types].xml", &(types + "</Types>"))?;
    add(
        "_rels/.rels",
        &format!(
            r#"<Relationships xmlns="{PACKAGE_REL}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
        ),
    )?;
    add("xl/_rels/workbook.xml.rels", &(rels + "</Relationships>"))?;
    add(
        "xl/workbook.xml",
        &format!(
            r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><workbookPr date1904="{}"/><sheets>{sheets}</sheets></workbook>"#,
            u8::from(date1904)
        ),
    )?;
    add(
        "xl/styles.xml",
        &format!(
            r#"<styleSheet xmlns="{MAIN}"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="22" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs></styleSheet>"#
        ),
    )?;
    zip.finish().map_err(|e| invalid(&e.to_string()))
}

fn axis(s: &str, origin: u32, max: u32) -> Option<(u32, bool, &str)> {
    let (coordinate, fixed, rest) = if let Some(relative) = s.strip_prefix('[') {
        let (offset, rest) = relative.split_once(']')?;
        (
            origin.checked_add_signed(offset.parse().ok()?)?,
            false,
            rest,
        )
    } else {
        let digits = s.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            (origin, false, s)
        } else {
            (s[..digits].parse().ok()?, true, &s[digits..])
        }
    };
    (1..=max)
        .contains(&coordinate)
        .then_some((coordinate, fixed, rest))
}

fn reference(s: &str, row: u32, column: u32) -> Option<(String, &str)> {
    let r = s.strip_prefix(['R', 'r'])?;
    let (row, fixed_row, c) = axis(r, row, ROWS)?;
    let (column, fixed_column, rest) = axis(c.strip_prefix(['C', 'c'])?, column, COLUMNS)?;
    if rest.starts_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '!' | '(')) {
        return None;
    }
    let dollar = |fixed| if fixed { "$" } else { "" };
    Some((
        format!(
            "{}{}{}{row}",
            dollar(fixed_column),
            column_name(column - 1),
            dollar(fixed_row)
        ),
        rest,
    ))
}

fn formula_to_a1(mut input: &str, row: u32, column: u32) -> String {
    let mut output = String::with_capacity(input.len());
    let mut quote = None;
    let mut word = false;
    while let Some(ch) = input.chars().next() {
        if quote.is_none()
            && !word
            && let Some((converted, rest)) = reference(input, row, column)
        {
            output.push_str(&converted);
            input = rest;
            word = true;
            continue;
        }
        if matches!(ch, '"' | '\'') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            }
        }
        word = ch.is_alphanumeric() || matches!(ch, '_' | '.');
        output.push(ch);
        input = &input[ch.len_utf8()..];
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<ss:Workbook xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet" xmlns:x="urn:schemas-microsoft-com:office:excel">
<x:ExcelWorkbook><x:Date1904/></x:ExcelWorkbook>
<ss:Worksheet ss:Name="First">
 <ss:Table ss:TopCell="2" ss:LeftCell="2">
  <ss:Row>
   <ss:Cell ss:MergeAcross="1"><ss:Data ss:Type="String"> 001 </ss:Data></ss:Cell>
   <ss:Cell><ss:Data ss:Type="Number">21</ss:Data></ss:Cell>
   <ss:Cell ss:Formula="=RC[-1]*2"><ss:Data ss:Type="Number">42</ss:Data></ss:Cell>
  </ss:Row>
  <ss:Row ss:Index="4"><ss:Cell ss:Index="2"><ss:Data ss:Type="DateTime">2024-01-01T12:00:00.000</ss:Data></ss:Cell></ss:Row>
 </ss:Table>
</ss:Worksheet>
<ss:Worksheet ss:Name="Second"><ss:Table><ss:Row><ss:Cell><ss:Data ss:Type="Boolean">1</ss:Data></ss:Cell></ss:Row></ss:Table></ss:Worksheet>
</ss:Workbook>"#;

    #[test]
    fn imports_utf8_and_utf16_workbooks_with_typed_sparse_cells() {
        let utf16 = [0xff, 0xfe]
            .into_iter()
            .chain(SAMPLE.encode_utf16().flat_map(u16::to_le_bytes))
            .collect::<Vec<_>>();
        for bytes in [SAMPLE.as_bytes(), &utf16] {
            let doc = Document::parse(SAMPLE).unwrap();
            let xlsx = to_xlsx(doc.root_element()).unwrap();
            let zip = crate::zip::Archive::new(&xlsx).unwrap();
            let xml =
                String::from_utf8(zip.read("xl/worksheets/sheet1.xml").unwrap().unwrap()).unwrap();
            for expected in [
                r#"r="B2" t="inlineStr""#,
                "> 001 </t>",
                r#"r="E2""#,
                "<f>D2*2</f>",
                r#"ref="B2:C2""#,
                r#"r="C5" t="n" s="1""#,
            ] {
                assert!(xml.contains(expected), "{expected}: {xml}");
            }
            let opened = try_open(bytes).unwrap().unwrap();
            assert_eq!(crate::Kind::of(&opened.presentation), crate::Kind::Workbook);
            assert_eq!(opened.presentation.slides.len(), 2);
        }
    }

    #[test]
    fn detects_the_namespaced_workbook_without_consuming_other_xml() {
        assert!(try_open(br#"<Workbook/>"#).unwrap().is_none());
        let invalid = SAMPLE.replace(r#"ss:Index="4""#, r#"ss:Index="1048577""#);
        assert!(try_open(invalid.as_bytes()).is_err());
        assert_eq!(
            formula_to_a1(r#"SUM('Q1'!R1C2:R[-1]C)+IF(RC[-1]="RC",1,0)"#, 4, 3),
            r#"SUM('Q1'!$B$1:C3)+IF(B4="RC",1,0)"#,
        );
    }
}
