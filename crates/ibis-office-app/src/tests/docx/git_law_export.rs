// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/235)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::Path;
use std::process::Command;

use crate::export::{Format, Options, export};
use ibis_doc_model::CellDirection;
use ibis_pdf::{PdfContainer, interpret::visit_text_scenes};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=.git/disabled-hooks",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub(super) fn run() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("repo");
    let old = Path::new("archive/old.lawtext");
    let new = Path::new("statutes/current.xml");
    std::fs::create_dir_all(root.join("archive")).unwrap();
    git(&root, &["init", "-q"]);
    std::fs::write(
        root.join(old),
        "試験法（令和七年法律第一号）\n第一条　赤色を用いる。\n",
    )
    .unwrap();
    git(&root, &["add", "--", "archive/old.lawtext"]);
    git(&root, &["commit", "-qm", "Old statute"]);
    let base = git(&root, &["rev-parse", "--short", "HEAD"]);
    git(&root, &["rm", "-q", "--", "archive/old.lawtext"]);
    std::fs::create_dir_all(root.join("statutes")).unwrap();
    std::fs::write(root.join(new), r#"<Law Era="Reiwa" Year="7" Num="1" LawType="Act" Lang="ja"><LawNum>令和七年法律第一号</LawNum><LawBody><LawTitle>試験法</LawTitle><MainProvision><Article Num="1"><ArticleTitle>第一条</ArticleTitle><Paragraph Num="1"><ParagraphNum/><ParagraphSentence><Sentence>青色を用いる。</Sentence></ParagraphSentence></Paragraph></Article></MainProvision></LawBody></Law>"#).unwrap();
    git(&root, &["add", "--", "statutes/current.xml"]);
    git(&root, &["commit", "-qm", "Move and amend statute"]);
    let head = git(&root, &["rev-parse", "HEAD"]);

    let docx = scratch.path().join("comparison.docx");
    crate::shell::scenario::save_git_law_comparison(
        &root,
        &base,
        old.into(),
        new.into(),
        docx.clone(),
    );
    let presentation = ibis_formats::read(&std::fs::read(&docx).unwrap()).unwrap();
    assert_eq!(
        presentation.flow.as_ref().unwrap().page.direction,
        CellDirection::TbRl
    );

    export(
        presentation,
        &Options {
            format: Format::Pdf,
            out_dir: scratch.path().to_owned(),
            name: "comparison".into(),
            width: 1920,
            slides: Vec::new(),
            layout: false,
            file_name: Some("comparison.docx".into()),
            print_backgrounds: false,
            print_hidden_text: false,
            print_markup: true,
            color: Default::default(),
            handout: None,
            frame: false,
            handout_landscape: false,
            notes_pages: false,
            outline_pages: false,
            notes_source: None,
            handout_marks: Default::default(),
        },
    )
    .unwrap();
    let mut pdf =
        PdfContainer::from_bytes(std::fs::read(scratch.path().join("comparison.pdf")).unwrap())
            .unwrap();
    let mut text = String::new();
    visit_text_scenes(&mut pdf, |page| {
        text.extend(
            page.text_index
                .plain_text()
                .chars()
                .filter(|ch| !ch.is_whitespace()),
        );
        Ok(())
    })
    .unwrap();
    for word in ["新旧対照条文", "改正案", "現行", "青色", "赤色"] {
        assert!(text.contains(word), "missing {word}: {text}");
    }
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
    assert!(git(&root, &["status", "--porcelain"]).is_empty());
}
