// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/235)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

#![cfg(not(target_family = "wasm"))]

use std::process::Command;

use ibis_formats::lawtext::{comparison_docx, parse_law_input};
use ibis_pdf::{PdfContainer, interpret::visit_text_scenes};

#[test]
fn long_vertical_comparison_keeps_short_supplement_on_one_pdf_page() {
    let repeats = 110;
    let passage = "協力を求めることができる。".repeat(repeats);
    let old = parse_law_input(&format!(
        "試験法（令和七年法律第一号）\n第一条　赤色を用いる。{passage}"
    ))
    .unwrap();
    let new = parse_law_input(&format!(
        "試験法（令和七年法律第一号）\n第一条　青色を用いる。{passage}\n附則\n１　公布の日から起算して六月を超えない範囲内において政令で定める日から施行する。"
    ))
    .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let document = directory.path().join("comparison.docx");
    std::fs::write(&document, comparison_docx(&old, &new).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ibisOffice"))
        .args(["--export", "pdf", "--out-dir"])
        .arg(directory.path())
        .arg(document)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut pdf =
        PdfContainer::from_bytes(std::fs::read(directory.path().join("comparison.pdf")).unwrap())
            .unwrap();
    let mut pages = Vec::new();
    visit_text_scenes(&mut pdf, |page| {
        pages.push(
            page.text_index
                .plain_text()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
        );
        Ok(())
    })
    .unwrap();

    let pages_containing = |text: &str| {
        pages
            .iter()
            .enumerate()
            .filter(|(_, page)| page.contains(text))
            .map(|(index, _)| index)
            .collect::<Vec<_>>()
    };
    assert!(
        pages_containing("協力").len() > 1,
        "the long row must span pages"
    );
    let printed_passages: usize = pages.iter().map(|page| page.matches('協').count()).sum();
    assert_eq!(
        printed_passages,
        2 * repeats,
        "both sides of the comparison must retain the full provision"
    );
    let heading = pages_containing("附則");
    assert_eq!(
        heading,
        [pages.len() - 1],
        "the supplement must have one page at the end"
    );
    assert_eq!(
        heading,
        pages_containing("公布の日"),
        "start of the supplementary provision"
    );
    assert_eq!(
        heading,
        pages_containing("施行する"),
        "end of the supplementary provision"
    );
    assert_eq!(
        heading,
        pages_containing("新設"),
        "comparison's current-law cell"
    );
}
