// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::{
    AbstractList, Block, FlowAlign, FlowDocument, FlowParagraph, FlowRun, FlowRunKind,
    ListInstance, ListLevel, ListSuffix, Numbering, PartKind, SectionSetup,
};

pub(super) fn add_list(numbering: &mut Numbering, levels: Vec<ListLevel>) -> u32 {
    let id = numbering.nums.len() as u32 + 1;
    numbering.abstracts.insert(
        id,
        AbstractList {
            levels,
            num_style_link: None,
        },
    );
    numbering.nums.insert(
        id,
        ListInstance {
            abstract_id: id,
            ..Default::default()
        },
    );
    id
}

pub(super) fn list_text(pattern: &str, level: u32) -> String {
    let mut output = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '^' {
            match chars.peek().copied() {
                Some(n @ '1'..='9') => {
                    chars.next();
                    output.push('%');
                    output.push(n);
                    continue;
                }
                Some(n @ ('n' | 'N')) => {
                    chars.next();
                    for depth in 1..=level.min(9) {
                        if depth != 1 {
                            output.push('.');
                        }
                        output.push('%');
                        output.push(char::from_digit(depth, 10).unwrap());
                    }
                    if n == 'N' {
                        output.push('.');
                    }
                    continue;
                }
                _ => {}
            }
        }
        output.push(ch);
    }
    output
}

pub(super) fn bullet_level(ch: char) -> ListLevel {
    let (text, symbol) = match ch {
        '\u{f06c}' => ('●', Some(("Wingdings".into(), ch))),
        '\u{f06e}' => ('■', Some(("Wingdings".into(), ch))),
        _ => (ch, None),
    };
    ListLevel {
        start: 1,
        format: "bullet".into(),
        text: text.to_string(),
        suffix: ListSuffix::Space,
        symbol,
        ..Default::default()
    }
}

pub(super) fn number_format(code: u32) -> &'static str {
    match code {
        1 => "decimalEnclosedCircle",
        2 => "upperRoman",
        3 => "lowerRoman",
        4 => "upperLetter",
        5 => "lowerLetter",
        8 => "ganada",
        10 => "chosung",
        12 => "koreanCounting",
        13 => "ideographDigital",
        _ => "decimal",
    }
}

pub(super) fn page_number(id: u32, align: FlowAlign, prefix: &str, suffix: &str) -> Block {
    let mut p = FlowParagraph {
        id,
        ..Default::default()
    };
    p.direct.align = Some(align);
    if !prefix.is_empty() {
        p.runs.push(FlowRun::new(
            FlowRunKind::Text(prefix.into()),
            Default::default(),
        ));
    }
    p.runs
        .push(FlowRun::new(FlowRunKind::PageNumber, Default::default()));
    if !suffix.is_empty() {
        p.runs.push(FlowRun::new(
            FlowRunKind::Text(suffix.into()),
            Default::default(),
        ));
    }
    p.resolve(&Default::default());
    Block::Paragraph(p)
}

pub(super) fn set_last_section(doc: &mut FlowDocument, section: SectionSetup) {
    doc.page = section.page;
    doc.restart_numbering = section.restart_numbering;
    doc.page_number_format = section.page_number_format;
    for (i, part) in section.parts.into_iter().enumerate() {
        doc.linked[i] = part.is_none();
        let target = match PartKind::ALL[i] {
            PartKind::Header => &mut doc.header,
            PartKind::Footer => &mut doc.footer,
            PartKind::HeaderFirst => &mut doc.header_first,
            PartKind::FooterFirst => &mut doc.footer_first,
            PartKind::HeaderEven => &mut doc.header_even,
            PartKind::FooterEven => &mut doc.footer_even,
        };
        *target = part.unwrap_or_default();
    }
}
