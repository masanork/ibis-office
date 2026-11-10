// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Conservative, read-only JTD text import. No OpenJTD code is used.
//!
//! Format observations: OpenJTD RFC 0003 and 0009 at cfb44c9ac53126e00808c49709fb4dade8a4daae,
//! https://github.com/OpenJTD/rjtd/tree/cfb44c9ac53126e00808c49709fb4dade8a4daae/openjtd-spec/rfc
//! Table candidates also use local RFC 0009 observations at 28927abb6b103dc70453f6443a7da961acfccb92.
//! Rule data follows the independent observations documented in docs/jtd-format-notes.md.
//! These are incomplete research notes, not a complete specification.
use crate::{Error, Kind};
use ibis_doc_model::{
    Block, BorderStyle, CellDirection, FlowBorder, FlowCell, FlowRow, FlowRun, FlowRunKind,
    FlowRunStyle, FlowTable, Presentation, Rgb, RubyAlign,
};

const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_STREAM: usize = 32 * 1024 * 1024;

/// Identify a supported JTD by its container and named document stream.
pub fn is_jtd(data: &[u8]) -> bool {
    checked_stream(data).is_ok_and(|s| s.starts_with(b"SsmgV.01"))
}

fn checked_stream(data: &[u8]) -> Result<Vec<u8>, Error> {
    if data.len() > MAX_FILE {
        return Err(Error::new("JTDファイルが大きすぎます"));
    }
    if data.len() < 512
        || !ibis_ooxml::cfb::is_cfb(data)
        || !matches!(u16::from_le_bytes([data[30], data[31]]), 9 | 12)
        || u16::from_le_bytes([data[32], data[33]]) != 6
    {
        return Err(Error::new("対応するJTDコンテナではありません"));
    }
    let stream = ibis_ooxml::cfb::stream(data, "DocumentText")
        .map_err(|_| Error::new("JTD本文がないか、暗号化・未対応の形式です"))?;
    if stream.len() > MAX_STREAM {
        return Err(Error::new("JTD本文が大きすぎます"));
    }
    Ok(stream)
}

/// Import only confirmed text into the existing document model. This is lossy
/// and must never be used as the source of a JTD save.
pub fn read(data: &[u8]) -> Result<Presentation, Error> {
    let stream = checked_stream(data)?;
    let segments = table_candidates(extract(&stream)?);
    let mut presentation = crate::read(&crate::blank_file(Kind::Document)?)?;
    let flow = std::sync::Arc::make_mut(
        presentation
            .flow
            .as_mut()
            .ok_or_else(|| Error::new("文書モデルを作成できません"))?,
    );
    let template = match flow.body.first() {
        Some(Block::Paragraph(p)) => p.clone(),
        _ => return Err(Error::new("文書の段落を作成できません")),
    };
    flow.body.clear();
    let mut paragraph = template.clone();
    paragraph.runs.clear();
    let mut next_id = 1u32;
    let mut last_table_key: Option<u16> = None;
    for segment in segments {
        match segment {
            Segment::Table {
                cells,
                total,
                strips,
                rules,
            } => {
                if !paragraph.runs.is_empty() || flow.body.is_empty() {
                    paragraph.id = next_id;
                    next_id += 1;
                    flow.body.push(Block::Paragraph(paragraph));
                    paragraph = template.clone();
                    paragraph.runs.clear();
                }
                let table_key = if strips { Some(total) } else { None };
                let total = total as i64;
                let width = (flow.page.size.width - flow.page.left - flow.page.right).max(1);
                let mut boundaries = vec![0i64];
                for pair in cells.windows(2) {
                    boundaries.push((pair[0].1 as i64 + pair[1].0 as i64) / 2);
                }
                boundaries.push(total);
                let line = FlowBorder {
                    width: 6_350,
                    color: Rgb::BLACK,
                    style: BorderStyle::Single,
                    space: 0,
                    art: None,
                };
                let mut borders = [Some(Some(line)); 6];
                // In this observed layout, the records describe successive
                // display lines, not proven logical table rows. Do not draw a
                // horizontal rule for each line merely because it has cells.
                if rules.is_some() {
                    borders = [Some(None); 6];
                } else if strips {
                    borders[4] = Some(None);
                }
                let mut table = FlowTable {
                    id: next_id,
                    borders,
                    columns: boundaries
                        .windows(2)
                        .map(|p| ((p[1] - p[0]) * width / total).max(635))
                        .collect(),
                    fit_to_page: true,
                    ..FlowTable::default()
                };
                next_id += 1;
                let mut row = FlowRow {
                    id: next_id,
                    ..FlowRow::default()
                };
                next_id += 1;
                for (left, right, contents) in cells {
                    let mut cell_para = template.clone();
                    cell_para.id = next_id;
                    next_id += 1;
                    cell_para.runs.clear();
                    for item in contents {
                        match item {
                            Segment::Text(text) => {
                                let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                                let mut pending = String::new();
                                for ch in normalized.chars() {
                                    if matches!(ch, '\n' | '\t') {
                                        if !pending.is_empty() {
                                            cell_para.runs.push(FlowRun::new(
                                                FlowRunKind::Text(std::mem::take(&mut pending)),
                                                FlowRunStyle::default(),
                                            ));
                                        }
                                        let kind = if ch == '\n' {
                                            FlowRunKind::LineBreak
                                        } else {
                                            FlowRunKind::Tab
                                        };
                                        cell_para
                                            .runs
                                            .push(FlowRun::new(kind, FlowRunStyle::default()));
                                    } else {
                                        pending.push(ch);
                                    }
                                }
                                if !pending.is_empty() {
                                    cell_para.runs.push(FlowRun::new(
                                        FlowRunKind::Text(pending),
                                        FlowRunStyle::default(),
                                    ));
                                }
                            }
                            Segment::Ruby { base, reading } => cell_para.runs.push(FlowRun::new(
                                FlowRunKind::Ruby {
                                    base,
                                    text: reading,
                                    size: 525,
                                    raise: 950,
                                    align: RubyAlign::Center,
                                },
                                FlowRunStyle::default(),
                            )),
                            Segment::PageBreak => cell_para.runs.push(FlowRun::new(
                                FlowRunKind::PageBreak,
                                FlowRunStyle::default(),
                            )),
                            _ => {}
                        }
                    }
                    let own_borders = rules.as_ref().map_or([None; 4], |rules| {
                        rule_cell_borders(rules, left, right, line)
                    });
                    row.cells.push(FlowCell {
                        id: next_id,
                        span: 1,
                        own_borders,
                        blocks: vec![Block::Paragraph(cell_para)],
                        ..FlowCell::default()
                    });
                    next_id += 1;
                }
                table.rows.push(row);
                table.resolve_borders();
                let mut combined = false;
                if paragraph.runs.is_empty() {
                    if let Some(Block::Table(previous)) = flow.body.last_mut() {
                        if previous.columns == table.columns && previous.borders == table.borders {
                            previous.rows.extend(table.rows.clone());
                            combined = true;
                        } else if table_key.is_some()
                            && table_key == last_table_key
                            && previous.borders == table.borders
                        {
                            combined = merge_table_grids(previous, &table);
                        }
                    }
                }
                if !combined {
                    flow.body.push(Block::Table(table));
                }
                last_table_key = table_key;
            }
            Segment::RowStart(_)
            | Segment::RowRules(_)
            | Segment::CellStart(_, _)
            | Segment::RowEnd => {
                last_table_key = None;
            }
            Segment::Text(text) => {
                last_table_key = None;
                let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                let mut lines = normalized.split('\n').peekable();
                while let Some(line) = lines.next() {
                    for (index, part) in line.split('\t').enumerate() {
                        if index > 0 {
                            paragraph
                                .runs
                                .push(FlowRun::new(FlowRunKind::Tab, FlowRunStyle::default()));
                        }
                        if !part.is_empty() {
                            paragraph.runs.push(FlowRun::new(
                                FlowRunKind::Text(part.to_string()),
                                FlowRunStyle::default(),
                            ));
                        }
                    }
                    if lines.peek().is_some() {
                        paragraph.id = next_id;
                        next_id = next_id
                            .checked_add(1)
                            .ok_or_else(|| Error::new("段落が多すぎます"))?;
                        flow.body.push(Block::Paragraph(paragraph));
                        paragraph = template.clone();
                        paragraph.runs.clear();
                    }
                }
            }
            Segment::PageBreak => {
                last_table_key = None;
                paragraph.runs.push(FlowRun::new(
                    FlowRunKind::PageBreak,
                    FlowRunStyle::default(),
                ));
            }
            Segment::Ruby { base, reading } => {
                last_table_key = None;
                // Typography is a Word default approximation; JTD's ruby
                // size and positioning have not yet been decoded.
                paragraph.runs.push(FlowRun::new(
                    FlowRunKind::Ruby {
                        base,
                        text: reading,
                        size: 525,
                        raise: 950,
                        align: RubyAlign::Center,
                    },
                    FlowRunStyle::default(),
                ));
            }
        }
    }
    paragraph.id = next_id;
    flow.body.push(Block::Paragraph(paragraph));
    flow.id_floor = next_id;
    // Candidate observed in independently generated horizontal/vertical pairs;
    // unsupported record layouts remain horizontal and are not guessed.
    if let Ok(styles) = ibis_ooxml::cfb::stream(data, "DocumentViewStyles") {
        if observed_vertical(&styles) {
            flow.page.direction = CellDirection::TbRl;
        }
    }
    Ok(presentation)
}

/// Align two adjacent observed display-line grids using spans. This retains
/// every original cell boundary; it does not infer logical JTD merges.
fn merge_table_grids(previous: &mut FlowTable, next: &FlowTable) -> bool {
    fn edges(columns: &[i64]) -> Option<Vec<i64>> {
        let mut out = vec![0i64];
        for &width in columns {
            out.push(out.last()?.checked_add(width)?);
        }
        Some(out)
    }
    let Some(a) = edges(&previous.columns) else {
        return false;
    };
    let Some(b) = edges(&next.columns) else {
        return false;
    };
    if a.last() != b.last() {
        return false;
    }
    let mut grid = a.clone();
    grid.extend_from_slice(&b);
    grid.sort_unstable();
    grid.dedup();
    if grid.len() > 33 {
        return false;
    }
    fn remap(rows: &mut [FlowRow], old: &[i64], grid: &[i64]) -> bool {
        for row in rows {
            let mut at = 0usize;
            for cell in &mut row.cells {
                let end = at + cell.span.max(1) as usize;
                if end >= old.len() {
                    return false;
                }
                let (Ok(start), Ok(stop)) =
                    (grid.binary_search(&old[at]), grid.binary_search(&old[end]))
                else {
                    return false;
                };
                cell.span = (stop - start) as u32;
                at = end;
            }
            if at != old.len() - 1 {
                return false;
            }
        }
        true
    }
    let mut merged = previous.clone();
    let mut incoming = next.rows.clone();
    if !remap(&mut merged.rows, &a, &grid) || !remap(&mut incoming, &b, &grid) {
        return false;
    }
    merged.columns = grid.windows(2).map(|pair| pair[1] - pair[0]).collect();
    merged.rows.extend(incoming);
    merged.resolve_borders();
    *previous = merged;
    true
}

/// Export the currently recognized model as DOCX. Unsupported JTD features
/// cannot be represented and the resulting file is not a lossless conversion.
pub fn export_docx(presentation: &Presentation) -> Result<Vec<u8>, Error> {
    let blank = crate::open(crate::blank_file(Kind::Document)?)?;
    blank
        .source
        .save(presentation, crate::SaveOptions::default())
}

/// The initial view-style record is an observed indicator, not a universal
/// specification. The enclosing header and record length must be valid.
fn observed_vertical(data: &[u8]) -> bool {
    if data.len() < 14 || data[..8] != [0, 1, 0, 2, 0x10, 0, 0, 0] {
        return false;
    }
    let group_len = u16::from_be_bytes([data[8], data[9]]) as usize;
    let kind = u16::from_be_bytes([data[10], data[11]]);
    let len = u16::from_be_bytes([data[12], data[13]]) as usize;
    kind == 0x1001 && group_len >= 4 && group_len <= data.len() - 10 && len <= group_len - 4
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    Text(String),
    Ruby {
        base: String,
        reading: String,
    },
    PageBreak,
    RowStart(u16),
    RowRules(RuleLayout),
    CellStart(u16, u16),
    RowEnd,
    Table {
        cells: Vec<(u16, u16, Vec<Segment>)>,
        total: u16,
        strips: bool,
        rules: Option<RuleLayout>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuleLayout {
    points: Vec<RulePoint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RulePoint {
    position: u16,
    flags: u16,
    partial: u16,
    link: u16,
}

/// Parse the documented 008f form only when all lengths and the final width
/// agree. Unknown variants are left to the existing conservative preview.
fn parse_rules(payload: &[u16]) -> Option<RuleLayout> {
    if payload.len() < 7 || payload[0] == 0 || payload[1] != 0 || !payload.ends_with(&[0xffff, 0]) {
        return None;
    }
    let width = payload[0] as u32;
    let mut position = u32::from(payload[2]).checked_add(1)?;
    let values = &payload[3..payload.len() - 2];
    if values.is_empty() || values.len() % 4 != 0 && values.len() % 4 != 2 {
        return None;
    }
    let mut points = Vec::new();
    let mut i = 0;
    while i < values.len() {
        if position > width || points.len() >= 64 {
            return None;
        }
        let (flags, partial, link, advance, step) = if values.len() - i == 2 {
            (values[i], 0, 0, u32::from(values[i + 1]) + 1, 2)
        } else {
            (
                values[i],
                values[i + 1],
                values[i + 2],
                u32::from(values[i + 1]) + u32::from(values[i + 3]) + 2,
                4,
            )
        };
        points.push(RulePoint {
            position: position.try_into().ok()?,
            flags,
            partial,
            link,
        });
        position = position.checked_add(advance)?;
        i += step;
    }
    (position == width).then_some(RuleLayout { points })
}

/// Map only whole-height and whole-cell lines. Mid-line, partial and
/// half-height rules need a different representation and are not invented.
fn rule_cell_borders(
    layout: &RuleLayout,
    left: u16,
    right: u16,
    line: FlowBorder,
) -> [Option<Option<FlowBorder>>; 4] {
    let nearest = |edge: u16| {
        layout
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.position.abs_diff(edge) <= 2)
            .min_by_key(|(_, p)| p.position.abs_diff(edge))
    };
    let l = nearest(left);
    let r = nearest(right);
    let full = |point: &RulePoint| point.flags & 0x30 != 0 && point.flags & 0x03 == 0x03;
    let bottom = match (l, r) {
        (Some((li, _)), Some((ri, _))) if ri > li => layout.points[li..ri]
            .iter()
            .all(|point| point.partial == 0 && point.link & 0x08 != 0),
        _ => false,
    };
    [
        Some(None),
        Some(r.and_then(|(_, p)| full(p).then_some(line))),
        Some(bottom.then_some(line)),
        Some(l.and_then(|(_, p)| full(p).then_some(line))),
    ]
}

fn extract(bytes: &[u8]) -> Result<Vec<Segment>, Error> {
    if !bytes.starts_with(b"SsmgV.01") || bytes.len() % 2 != 0 {
        return Err(Error::new("JTD本文形式が未対応か壊れています"));
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .collect();
    let mut out = Vec::new();
    // Observed TextV.01 plain-text record: 32-bit BE number of UTF-16 units.
    if bytes.get(20..28) == Some(b"TextV.01") && bytes.len() >= 32 {
        let count = u32::from_be_bytes(bytes[28..32].try_into().unwrap()) as usize;
        if count > 0 && count <= (bytes.len() - 32) / 2 && units.get(16).is_some_and(|x| *x >= 0x20)
        {
            let s = String::from_utf16(&units[16..16 + count])
                .map_err(|_| Error::new("JTD文字列が壊れています"))?;
            if s.chars()
                .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
            {
                out.push(Segment::Text(s));
            }
        }
    }
    if out.is_empty() {
        let mut i = 4;
        while i + 7 <= units.len() {
            if units[i] == 0x000e {
                out.push(Segment::RowEnd);
                i += 1;
                continue;
            }
            if units[i] != 0x001c {
                i += 1;
                continue;
            }
            let class = units[i + 1];
            let count = units[i + 2] as usize;
            if class == 1 && count == 7 && units[i + 6] == 0x001d {
                // Only the observed visible base/placeholder selectors. Other
                // inline payloads may be ruby or control data, so omit them.
                let visible = units[i + 4] == 0 && matches!(units[i + 5], 1 | 3);
                let start = i + 7;
                let end = (start..units.len().min(start + 256)).find(|&n| units[n] == 0x001e);
                if let Some(end) = end {
                    if visible {
                        let value = String::from_utf16(&units[start..end])
                            .map_err(|_| Error::new("JTD文字列が壊れています"))?;
                        if value
                            .chars()
                            .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
                        {
                            // Correlated in local files: the base is followed
                            // by the four-word separator and a ruby annotation.
                            let next = end + 1;
                            let paired = units.get(next..next + 11)
                                == Some(&[5, 0, 1, 0x1f, 0x1c, 1, 7, 0, 1, 0x82, 0x1d]);
                            if paired {
                                let reading_start = next + 11;
                                let reading_end = (reading_start
                                    ..units.len().min(reading_start + 256))
                                    .find(|&n| units[n] == 0x001e);
                                if let Some(reading_end) = reading_end {
                                    let reading =
                                        String::from_utf16(&units[reading_start..reading_end])
                                            .map_err(|_| Error::new("JTDルビが壊れています"))?;
                                    if !value.is_empty()
                                        && !reading.is_empty()
                                        && !value.chars().any(char::is_control)
                                        && !reading.chars().any(char::is_control)
                                    {
                                        out.push(Segment::Ruby {
                                            base: value,
                                            reading,
                                        });
                                        i = reading_end + 1;
                                        continue;
                                    }
                                }
                            }
                            out.push(Segment::Text(value));
                        }
                    }
                    i = end + 1;
                    continue;
                }
            }
            let end = match i.checked_add(count) {
                Some(end) if count >= 7 && end <= units.len() => end,
                _ => {
                    i += 1;
                    continue;
                }
            };
            if !matches!(class, 0 | 0x10 | 0x20 | 0x30)
                || units[end - 4] as usize != count
                || units[end - 3] != 0
                || units[end - 2] != class
                || units[end - 1] != 0x001f
            {
                i += 1;
                continue;
            }
            if class == 0x10
                && count >= 12
                && units[i + 4] == 0x008f
                && units[i + 5] as usize + 12 == count
                && units[i + 6] > 0
            {
                out.push(Segment::RowStart(units[i + 6]));
                if let Some(rules) = parse_rules(&units[i + 6..end - 4]) {
                    out.push(Segment::RowRules(rules));
                }
            } else if class == 0x30
                && count == 12
                && units[i + 3] == 0
                && units[i + 6] == 0xff
                && units[i + 7] == 0
            {
                out.push(Segment::CellStart(units[i + 4], units[i + 5]));
            }
            i = end;
            let start = i;
            while i < units.len()
                && (units[i] >= 0x00a0
                    || matches!(units[i], 0x09 | 0x0a | 0x0d)
                    || (0x20..0x7f).contains(&units[i]))
            {
                i += 1;
            }
            if start != i {
                let text = String::from_utf16(&units[start..i])
                    .map_err(|_| Error::new("JTD文字列が壊れています"))?;
                out.push(Segment::Text(text));
                // An observed break directly after body text. Do not infer
                // break semantics from the same value inside binary headers.
                if units.get(i) == Some(&0x000c) {
                    out.push(Segment::PageBreak);
                    i += 1;
                }
            }
        }
    }
    if !out.iter().any(|segment| match segment {
        Segment::Text(text) => !text.trim().is_empty(),
        Segment::Ruby { base, .. } => !base.trim().is_empty(),
        Segment::PageBreak
        | Segment::RowStart(_)
        | Segment::RowRules(_)
        | Segment::CellStart(_, _)
        | Segment::RowEnd
        | Segment::Table { .. } => false,
    }) {
        return Err(Error::new("対応するJTD本文を読み取れませんでした"));
    }
    Ok(out)
}

/// Reconstruct only complete, ordered cell runs. Borders and physical widths
/// are preview approximations; source stroke styles and units remain unknown.
fn table_candidates(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut iter = segments.into_iter().peekable();
    while let Some(segment) = iter.next() {
        let Segment::RowStart(total) = segment else {
            out.push(segment);
            continue;
        };
        let mut original = vec![Segment::RowStart(total)];
        while let Some(next) = iter.peek() {
            if matches!(next, Segment::RowStart(_)) {
                break;
            }
            let end = matches!(next, Segment::RowEnd);
            original.push(iter.next().unwrap());
            if end {
                break;
            }
        }
        let mut cells: Vec<(u16, u16, Vec<Segment>)> = Vec::new();
        let mut rules = None;
        let mut valid = matches!(original.last(), Some(Segment::RowEnd));
        for part in &original[1..] {
            match part {
                Segment::CellStart(a, b) => {
                    if a >= b
                        || *b > total
                        || cells.last().is_some_and(|c| c.1 > *a)
                        || cells.len() >= 64
                    {
                        valid = false;
                    }
                    cells.push((*a, *b, Vec::new()));
                }
                Segment::RowRules(value) if cells.is_empty() && rules.is_none() => {
                    rules = Some(value.clone());
                }
                Segment::RowEnd => {}
                Segment::Text(_) | Segment::Ruby { .. } | Segment::PageBreak => {
                    if let Some(cell) = cells.last_mut() {
                        cell.2.push(part.clone());
                    } else {
                        valid = false;
                    }
                }
                _ => valid = false,
            }
        }
        let visible = cells.iter().any(|c| {
            c.2.iter().any(|s| {
                matches!(s, Segment::Text(t) if !t.trim().is_empty())
                    || matches!(s, Segment::Ruby { .. })
            })
        });
        if valid
            && (cells.len() >= 2 && visible || rules.is_some() && !cells.is_empty() && !visible)
        {
            // Some empty ruled lines contain only the edge strips. Preserve
            // their empty interior so a line beneath it has a cell to attach
            // to; otherwise the top edge of the following box disappears.
            if !visible && rules.is_some() && cells.iter().all(|cell| cell.2.is_empty()) {
                let mut expanded: Vec<(u16, u16, Vec<Segment>)> =
                    Vec::with_capacity(cells.len() * 2);
                for cell in cells {
                    if let Some((_, prior_end, _)) = expanded.last() {
                        if let Some(gap_start) = prior_end.checked_add(4) {
                            if let Some(gap_end) = cell.0.checked_sub(4) {
                                if gap_start < gap_end {
                                    expanded.push((gap_start, gap_end, Vec::new()));
                                }
                            }
                        }
                    }
                    expanded.push(cell);
                }
                cells = expanded;
            }
            // Thin, empty outer strips in the paired comparison-table
            // pattern carry edge geometry, not independent text columns.
            // An empty two-unit strip inside the same bounded pattern is
            // likewise a boundary candidate, not a text column. Keep every
            // content-bearing cell, including whitespace and controls.
            let empty = |items: &[Segment]| {
                items.iter().all(|part| match part {
                    Segment::Text(t) => t.trim().is_empty(),
                    _ => false,
                })
            };
            let first = &cells[0];
            let last = cells.last().unwrap();
            let strips = (cells.len() >= 4 || !visible && rules.is_some() && cells.len() >= 3)
                && first.0 == 0
                && first.1 <= 6
                && last.1 == total
                && last.1 - last.0 <= 6
                && empty(&first.2)
                && empty(&last.2);
            if strips {
                cells.pop();
                cells.remove(0);
                if cells
                    .iter()
                    .filter(|(left, right, content)| right - left > 2 || !content.is_empty())
                    .count()
                    >= 2
                {
                    cells.retain(|(left, right, content)| right - left > 2 || !content.is_empty());
                }
            }
            out.push(Segment::Table {
                cells,
                total,
                strips,
                rules,
            });
        } else {
            out.extend(original);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_header_maps_only_complete_cell_edges_and_survives_docx() {
        let mut units: Vec<u16> = vec![
            0x1c, 0x10, 27, 0, 0x8f, 15, 100, 0, 3, 0x2b, 0, 8, 42, 0x23, 0, 0, 42, 0x23, 0, 0, 6,
            0xffff, 0, 27, 0, 0x10, 0x1f,
        ];
        units.extend([0x1c, 0x30, 12, 0, 0, 2, 0xff, 0, 12, 0, 0x30, 0x1f]);
        units.extend([0x1c, 0x30, 12, 0, 6, 46, 0xff, 0, 12, 0, 0x30, 0x1f]);
        units.extend("甲".encode_utf16());
        units.extend([0x1c, 0x30, 12, 0, 50, 90, 0xff, 0, 12, 0, 0x30, 0x1f]);
        units.extend("乙".encode_utf16());
        units.extend([0x1c, 0x30, 12, 0, 94, 100, 0xff, 0, 12, 0, 0x30, 0x1f]);
        units.extend([0x0e, 0, 0, 0, 0, 0, 0, 0]);
        let mut stream = b"SsmgV.01".to_vec();
        for unit in units {
            stream.extend_from_slice(&unit.to_be_bytes());
        }
        let input =
            ibis_ooxml::cfb::write(&[ibis_ooxml::cfb::Item::stream("DocumentText", stream)]);
        let imported = read(&input).unwrap();
        let saved = crate::read(&export_docx(&imported).unwrap()).unwrap();
        for doc in [&imported, &saved] {
            let table = doc
                .flow
                .as_ref()
                .unwrap()
                .body
                .iter()
                .find_map(|b| match b {
                    Block::Table(t) => Some(t),
                    _ => None,
                })
                .unwrap();
            assert_eq!(table.rows[0].cells.len(), 2);
            let a = &table.rows[0].cells[0];
            let b = &table.rows[0].cells[1];
            assert!(a.borders[3].is_some() && a.borders[1].is_some());
            assert!(a.borders[2].is_some());
            assert!(b.borders[2].is_none());
            assert!(b.borders[1].is_some());
        }
    }

    #[test]
    fn malformed_rule_records_are_not_promoted() {
        let good = [
            100, 0, 3, 0x2b, 0, 8, 42, 0x23, 0, 0, 42, 0x23, 0, 0, 6, 0xffff, 0,
        ];
        let layout = parse_rules(&good).unwrap();
        assert_eq!(
            layout.points.iter().map(|p| p.position).collect::<Vec<_>>(),
            [4, 48, 92]
        );
        let short_last = [21, 0, 0, 0x13, 0, 0, 8, 0x13, 9, 0xffff, 0];
        assert_eq!(
            parse_rules(&short_last)
                .unwrap()
                .points
                .iter()
                .map(|p| p.position)
                .collect::<Vec<_>>(),
            [1, 11]
        );
        let mut bad = good;
        bad[0] = 99;
        assert!(parse_rules(&bad).is_none());
        bad = good;
        bad[15] = 0;
        assert!(parse_rules(&bad).is_none());
        assert!(parse_rules(&good[..10]).is_none());
    }

    #[test]
    fn empty_rule_line_keeps_its_interior_and_bottom_edge() {
        let layout = RuleLayout {
            points: vec![
                RulePoint {
                    position: 4,
                    flags: 0x2b,
                    partial: 0,
                    link: 8,
                },
                RulePoint {
                    position: 96,
                    flags: 0x23,
                    partial: 0,
                    link: 0,
                },
            ],
        };
        let segments = table_candidates(vec![
            Segment::RowStart(100),
            Segment::RowRules(layout.clone()),
            Segment::CellStart(0, 2),
            Segment::CellStart(98, 100),
            Segment::RowEnd,
        ]);
        let [
            Segment::Table {
                cells,
                rules: Some(parsed),
                ..
            },
        ] = segments.as_slice()
        else {
            panic!("expected empty ruled table")
        };
        assert_eq!(cells.len(), 1);
        assert_eq!((cells[0].0, cells[0].1), (6, 94));
        let line = FlowBorder {
            width: 6350,
            color: Rgb::BLACK,
            style: BorderStyle::Single,
            space: 0,
            art: None,
        };
        assert!(
            rule_cell_borders(parsed, cells[0].0, cells[0].1, line)[2]
                .unwrap()
                .is_some()
        );
        let mut partial = layout;
        partial.points[0].partial = 1;
        assert!(
            rule_cell_borders(&partial, 6, 94, line)[2]
                .unwrap()
                .is_none()
        );
        partial.points[0].flags = 0x21;
        partial.points[0].link = 0x24;
        let borders = rule_cell_borders(&partial, 6, 94, line);
        assert!(
            borders[3].unwrap().is_none(),
            "half-height is not a full cell edge"
        );
        assert!(
            borders[2].unwrap().is_none(),
            "a middle line is not a bottom edge"
        );
    }
    fn fixture(text: &str) -> Vec<u8> {
        let mut s = b"SsmgV.01".to_vec();
        for n in [0x001c_u16, 0x0010, 7, 7, 0, 0x0010, 0x001f]
            .into_iter()
            .chain(text.encode_utf16())
            .chain([0u16])
        {
            s.extend_from_slice(&n.to_be_bytes());
        }
        ibis_ooxml::cfb::write(&[ibis_ooxml::cfb::Item::stream("DocumentText", s)])
    }
    #[test]
    fn reads_text_and_does_not_treat_jtd_as_xls() {
        let bytes = fixture("日本語\nsecond");
        assert!(is_jtd(&bytes));
        let result = crate::read(&bytes).unwrap();
        let flow = result.flow.unwrap();
        assert_eq!(flow.body.len(), 2);
        assert!(matches!(&flow.body[0], Block::Paragraph(p) if p.text() == "日本語"));
        assert!(matches!(&flow.body[1], Block::Paragraph(p) if p.text() == "second"));
    }
    #[test]
    fn exports_recognized_text_and_page_direction_as_docx() {
        let mut presentation = read(&fixture("日本語\nsecond")).unwrap();
        std::sync::Arc::make_mut(presentation.flow.as_mut().unwrap())
            .page
            .direction = CellDirection::TbRl;
        let data = export_docx(&presentation).unwrap();
        let reopened = crate::read(&data).unwrap();
        let flow = reopened.flow.unwrap();
        assert!(flow.page.direction.vertical());
        assert!(matches!(&flow.body[0], Block::Paragraph(p) if p.text() == "日本語"));
        assert!(matches!(&flow.body[1], Block::Paragraph(p) if p.text() == "second"));
    }

    #[test]
    fn observed_table_cells_export_with_approximate_grid() {
        let mut units: Vec<u16> = vec![
            0x1c, 0x10, 27, 0, 0x8f, 15, 100, 0, 3, 0x23, 0, 0, 48, 0x23, 0, 0, 48, 0x23, 0, 0, 2,
            0xffff, 0, 27, 0, 0x10, 0x1f,
        ];
        units.extend([0x1c, 0x30, 12, 0, 0, 48, 0xff, 0, 12, 0, 0x30, 0x1f]);
        units.extend("甲".encode_utf16());
        units.extend([0x1c, 0x30, 12, 0, 52, 100, 0xff, 0, 12, 0, 0x30, 0x1f]);
        units.extend("乙".encode_utf16());
        units.extend([0x0e, 0, 0, 0, 0, 0, 0, 0]);
        let mut stream = b"SsmgV.01".to_vec();
        for u in units {
            stream.extend_from_slice(&u.to_be_bytes());
        }
        let input =
            ibis_ooxml::cfb::write(&[ibis_ooxml::cfb::Item::stream("DocumentText", stream)]);
        let doc = read(&input).unwrap();
        let flow = doc.flow.as_ref().unwrap();
        let table = flow
            .body
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("table");
        assert_eq!(table.rows[0].cells.len(), 2);
        assert!(table.borders.iter().all(|b| b.is_some()));
        let reopened = crate::read(&export_docx(&doc).unwrap()).unwrap();
        let saved = reopened.flow.unwrap();
        let table = saved
            .body
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("saved table");
        assert_eq!(table.rows[0].cells.len(), 2);
        assert!(
            matches!(&table.rows[0].cells[0].blocks[0], Block::Paragraph(p) if p.text() == "甲")
        );
        assert!(
            matches!(&table.rows[0].cells[1].blocks[0], Block::Paragraph(p) if p.text() == "乙")
        );
        assert!(table.borders.iter().all(|b| b.is_some()));
    }

    #[test]
    fn incomplete_or_overlapping_rows_remain_text() {
        let original = vec![
            Segment::RowStart(100),
            Segment::CellStart(0, 70),
            Segment::Text("a".into()),
            Segment::CellStart(60, 100),
            Segment::Text("b".into()),
            Segment::RowEnd,
        ];
        assert_eq!(table_candidates(original.clone()), original);
        assert_eq!(
            table_candidates(vec![
                Segment::RowStart(100),
                Segment::CellStart(0, 100),
                Segment::Text("a".into())
            ])
            .iter()
            .filter(|s| matches!(s, Segment::Table { .. }))
            .count(),
            0
        );
    }

    #[test]
    fn paired_empty_edge_strips_are_not_text_columns() {
        let row = vec![
            Segment::RowStart(100),
            Segment::CellStart(0, 2),
            Segment::CellStart(6, 48),
            Segment::Text("left".into()),
            Segment::CellStart(52, 90),
            Segment::Text("right".into()),
            Segment::CellStart(94, 100),
            Segment::RowEnd,
        ];
        let result = table_candidates(row.clone());
        assert!(
            matches!(&result[..], [Segment::Table { cells, total: 100, strips: true, .. }] if cells.len() == 2)
        );
        let mut with_edge_text = row;
        with_edge_text.insert(2, Segment::Text("edge".into()));
        let result = table_candidates(with_edge_text);
        assert!(
            matches!(&result[..], [Segment::Table { cells, strips: false, .. }] if cells.len() == 4)
        );
    }

    #[test]
    fn empty_narrow_inner_strips_are_removed_without_dropping_content() {
        let row = vec![
            Segment::RowStart(100),
            Segment::CellStart(0, 2),
            Segment::CellStart(6, 30),
            Segment::Text("A".into()),
            Segment::CellStart(34, 36),
            Segment::CellStart(40, 42),
            Segment::Text("※".into()),
            Segment::CellStart(46, 90),
            Segment::Text("B".into()),
            Segment::CellStart(94, 100),
            Segment::RowEnd,
        ];
        let result = table_candidates(row);
        assert!(
            matches!(&result[..], [Segment::Table { cells, strips: true, .. }]
            if cells.len() == 3 && cells[1].0 == 40 && matches!(&cells[1].2[..], [Segment::Text(t)] if t == "※"))
        );
    }

    #[test]
    fn adjacent_grids_preserve_cell_boundaries_as_spans() {
        fn table(columns: &[i64]) -> FlowTable {
            FlowTable {
                columns: columns.to_vec(),
                rows: vec![FlowRow {
                    cells: columns
                        .iter()
                        .map(|_| FlowCell {
                            span: 1,
                            ..FlowCell::default()
                        })
                        .collect(),
                    ..FlowRow::default()
                }],
                ..FlowTable::default()
            }
        }
        let mut first = table(&[50, 50]);
        assert!(merge_table_grids(&mut first, &table(&[25, 25, 50])));
        assert_eq!(first.columns, [25, 25, 50]);
        assert_eq!(
            first.rows[0]
                .cells
                .iter()
                .map(|c| c.span)
                .collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(
            first.rows[1]
                .cells
                .iter()
                .map(|c| c.span)
                .collect::<Vec<_>>(),
            [1, 1, 1]
        );
        let before = first.clone();
        assert!(!merge_table_grids(&mut first, &table(&[40, 40])));
        assert_eq!(first, before);
    }

    #[test]
    fn view_direction_needs_valid_evidence() {
        let mut record = vec![0, 1, 0, 2, 0x10, 0, 0, 0, 0, 6, 0x10, 1, 0, 2, 0, 0];
        assert!(observed_vertical(&record));
        record[11] = 2;
        assert!(!observed_vertical(&record));
        record[11] = 1;
        record[13] = 3;
        assert!(!observed_vertical(&record));
        assert!(!observed_vertical(&record[..12]));
    }

    #[test]
    fn observed_page_break_is_preserved_in_docx() {
        let mut s = b"SsmgV.01".to_vec();
        for unit in [
            0x1c_u16, 0x10, 7, 7, 0, 0x10, 0x1f, 0x7532, 0x000c, 0x1c, 0x10, 7, 7, 0, 0x10, 0x1f,
            0x4e59,
        ] {
            s.extend_from_slice(&unit.to_be_bytes());
        }
        let data = ibis_ooxml::cfb::write(&[ibis_ooxml::cfb::Item::stream("DocumentText", s)]);
        let model = read(&data).unwrap();
        assert!(
            matches!(&model.flow.as_ref().unwrap().body[0], Block::Paragraph(p) if p.runs.iter().any(|r| matches!(r.kind, FlowRunKind::PageBreak)))
        );
        let reopened = crate::read(&export_docx(&model).unwrap()).unwrap();
        assert!(
            matches!(&reopened.flow.unwrap().body[0], Block::Paragraph(p) if p.runs.iter().any(|r| matches!(r.kind, FlowRunKind::PageBreak)))
        );
    }

    #[test]
    fn observed_ruby_pair_is_kept_in_word_model_and_docx() {
        let mut s = b"SsmgV.01".to_vec();
        for unit in [
            0x1c_u16, 1, 7, 0, 0, 3, 0x1d, 0x6f22, 0x1e, 5, 0, 1, 0x1f, 0x1c, 1, 7, 0, 1, 0x82,
            0x1d, 0x304b, 0x3093, 0x1e,
        ] {
            s.extend_from_slice(&unit.to_be_bytes());
        }
        let data = ibis_ooxml::cfb::write(&[ibis_ooxml::cfb::Item::stream("DocumentText", s)]);
        let model = read(&data).unwrap();
        let flow = model.flow.as_ref().unwrap();
        assert!(
            matches!(&flow.body[0], Block::Paragraph(p) if matches!(&p.runs[0].kind, FlowRunKind::Ruby {base, text, ..} if base == "漢" && text == "かん"))
        );
        let docx = export_docx(&model).unwrap();
        let reopened = crate::read(&docx).unwrap();
        assert!(
            matches!(&reopened.flow.unwrap().body[0], Block::Paragraph(p) if p.runs.iter().any(|r| matches!(&r.kind, FlowRunKind::Ruby {base, text, ..} if base == "漢" && text == "かん")))
        );
    }

    #[test]
    fn raw_text_and_inline_selectors_are_bounded() {
        let mut raw = b"SsmgV.01".to_vec();
        raw.extend_from_slice(&[0; 12]);
        raw.extend_from_slice(b"TextV.01");
        raw.extend_from_slice(&2u32.to_be_bytes());
        for unit in "文章".encode_utf16() {
            raw.extend_from_slice(&unit.to_be_bytes());
        }
        assert_eq!(extract(&raw).unwrap(), vec![Segment::Text("文章".into())]);
        let mut data = b"SsmgV.01".to_vec();
        for unit in [
            0x1c_u16, 1, 7, 0, 0, 3, 0x1d, 0x6587, 0x001e, 0x1c, 1, 7, 0, 1, 0x82, 0x1d, 0x3088,
            0x001e,
        ] {
            data.extend_from_slice(&unit.to_be_bytes());
        }
        assert_eq!(extract(&data).unwrap(), vec![Segment::Text("文".into())]);
    }

    #[test]
    #[ignore = "requires a locally authorized JTD sample"]
    fn local_sample_can_be_read() {
        let path = std::env::var_os("IBIS_JTD_SAMPLE").expect("IBIS_JTD_SAMPLE");
        let bytes = std::fs::read(path).unwrap();
        if std::env::var_os("IBIS_JTD_TABLE_STATS").is_some() {
            let mut stats = std::collections::BTreeMap::new();
            for segment in table_candidates(extract(&checked_stream(&bytes).unwrap()).unwrap()) {
                if let Segment::Table { cells, .. } = segment {
                    for (left, right, content) in cells {
                        let visible = content.iter().any(|s| match s {
                            Segment::Text(t) => !t.trim().is_empty(),
                            Segment::Ruby { .. } => true,
                            _ => false,
                        });
                        *stats
                            .entry((right - left, visible, content.is_empty()))
                            .or_insert(0usize) += 1;
                    }
                }
            }
            eprintln!("cell width, visible: {stats:?}");
        }
        let result = read(&bytes).unwrap();
        let word = export_docx(&result).unwrap();
        if let Some(path) = std::env::var_os("IBIS_JTD_DOCX_OUT") {
            std::fs::write(path, &word).unwrap();
        }
        let reopened = crate::read(&word).unwrap();
        let flow = result.flow.unwrap();
        let saved = reopened.flow.unwrap();
        assert_eq!(flow.page.direction, saved.page.direction);
        fn visit(
            blocks: &[Block],
            texts: &mut Vec<String>,
            rubies: &mut usize,
            tables: &mut usize,
        ) {
            for block in blocks {
                match block {
                    Block::Paragraph(p) => {
                        let mut plain = String::new();
                        for run in &p.runs {
                            match &run.kind {
                                FlowRunKind::Text(t) => plain.push_str(t),
                                FlowRunKind::Ruby { base, .. } => plain.push_str(base),
                                FlowRunKind::Tab => plain.push('\t'),
                                FlowRunKind::LineBreak => plain.push('\n'),
                                _ => {}
                            }
                        }
                        texts.push(plain);
                        *rubies += p
                            .runs
                            .iter()
                            .filter(|run| matches!(run.kind, FlowRunKind::Ruby { .. }))
                            .count();
                    }
                    Block::Table(t) => {
                        *tables += 1;
                        for row in &t.rows {
                            for cell in &row.cells {
                                visit(&cell.blocks, texts, rubies, tables);
                            }
                        }
                    }
                }
            }
        }
        let (mut texts, mut rubies, mut tables) = (Vec::new(), 0, 0);
        let (mut saved_texts, mut saved_rubies, mut saved_tables) = (Vec::new(), 0, 0);
        visit(&flow.body, &mut texts, &mut rubies, &mut tables);
        visit(
            &saved.body,
            &mut saved_texts,
            &mut saved_rubies,
            &mut saved_tables,
        );
        assert_eq!(rubies, saved_rubies);
        assert_eq!(tables, saved_tables);
        if let Ok(minimum) = std::env::var("IBIS_JTD_MIN_RUBY") {
            assert!(rubies >= minimum.parse().unwrap());
        }
        if let Ok(minimum) = std::env::var("IBIS_JTD_MIN_TABLES") {
            assert!(tables >= minimum.parse().unwrap(), "tables: {tables}");
        }
        let text = texts.join("\n");
        assert!(!text.trim().is_empty());
        assert_eq!(text, saved_texts.join("\n"));
        // Grouping cells must not silently drop or duplicate imported text.
        let source_text: String = extract(&checked_stream(&bytes).unwrap())
            .unwrap()
            .into_iter()
            .filter_map(|s| match s {
                Segment::Text(t) => Some(t),
                Segment::Ruby { base, .. } => Some(base),
                _ => None,
            })
            .collect();
        assert_eq!(
            source_text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
            text.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        );
        if let Ok(expected) = std::env::var("IBIS_JTD_EXPECT") {
            assert!(text.contains(&expected));
        }
        if let Ok(direction) = std::env::var("IBIS_JTD_DIRECTION") {
            assert_eq!(flow.page.direction.vertical(), direction == "vertical");
        }
    }
    #[test]
    fn rejects_invalid_and_unsupported() {
        assert!(read(&[]).is_err());
        let b = ibis_ooxml::cfb::write(&[ibis_ooxml::cfb::Item::stream(
            "DocumentText",
            b"SsmgV.01".to_vec(),
        )]);
        assert!(read(&b).is_err());
        let mut b = fixture("test");
        b[30] = 255;
        assert!(read(&b).is_err());
    }
}
