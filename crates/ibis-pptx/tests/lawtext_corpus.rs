// SPDX-License-Identifier: AGPL-3.0-only
use ibis_pptx::lawtext::{input_to_markdown, parse_law_xml, parse_lawtext, to_lawtext};

#[test]
fn egov_constitution_opens_as_markdown_and_lawtext() {
    let xml = include_str!("lawtext/321CONSTITUTION.xml");
    let law = parse_law_xml(xml).unwrap();
    assert_eq!(law.law_title, "日本国憲法");
    let markdown = input_to_markdown(xml.as_bytes()).unwrap().unwrap();
    assert!(markdown.contains("われらとわれらの子孫のために"));
    let lawtext = to_lawtext(&law);
    let reparsed = parse_lawtext(&lawtext).unwrap();
    assert_eq!(reparsed.law_title, law.law_title);
    assert_eq!(reparsed.main_provision, law.main_provision);
}
