// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Open and save without an edit, by the window's own path (review 0.8.5, 「開いた直後にフィールドの更新などでモデルを書き換える」).
//! `crates/ibis-formats/tests/roundtrip_corpus.rs` opens the bytes and saves the model as it was read. The window does more as it
//! opens a file (`SlideEditor::load`: fields, tables of contents, indexes, pages, pivots, row heights, dates), so a file nobody
//! edited could be saved as another file, or not be saved. Here every public test file is opened as the window opens it, saved
//! as 保存 saves it, and the saved file is compared with the original part by part. The result goes to
//! `target/roundtrip/app-result.tsv`.
use crate::shell::scenario::Scenario;

pub(crate) fn all() -> Vec<(&'static str, fn())> {
    vec![
        ("shell::markdown_desktop_roundtrip", markdown_desktop_roundtrip),
        ("shell::open_and_save_without_an_edit_keeps_every_part", open_and_save_without_an_edit_keeps_every_part),
        ("shell::save_reports_markdown_source_loss", save_reports_markdown_source_loss),
        ("shell::save_image_preview_keeps_transparent_pixels", save_image_preview_keeps_transparent_pixels),
    ]
}

/// (file name, why it differs and where it is tracked): defects not mended yet.
const KNOWN: &[(&str, &str)] = &[];

fn files() -> Vec<std::path::PathBuf> {
    fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["docx", "xlsx", "pptx"].contains(&e.to_ascii_lowercase().as_str())) {
                out.push(path);
            }
        }
    }
    let root = crate::shell::scenario::repo_root().join("test");
    let mut found = Vec::new();
    collect(&root.join("editing"), &mut found);
    collect(&root.join("benchmark"), &mut found);
    for dir in ["docx", "xlsx", "pptx"] {
        collect(&root.join("corpus-report").join(dir), &mut found);
    }
    if let Ok(dirs) = std::env::var("IBIS_ROUNDTRIP_DIRS") {
        for dir in dirs.split(':').filter(|d| !d.is_empty()) {
            collect(std::path::Path::new(dir), &mut found);
        }
    }
    found.sort();
    found.dedup();
    if let Some(only) = std::env::var("IBIS_ROUNDTRIP_ONLY").ok().filter(|o| !o.is_empty()) {
        found.retain(|p| p.to_string_lossy().contains(&only));
    }
    found
}

fn open_and_save_without_an_edit_keeps_every_part() {
    let files = files();
    let mut s = Scenario::open("test/editing/docx-report.docx");
    let out = crate::shell::scenario::repo_root().join("target/roundtrip");
    let _ = std::fs::create_dir_all(&out);
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut checked = 0;
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Ok(original) = std::fs::read(path) else { continue };
        let copy = out.join(format!("app-{name}"));
        let _ = std::fs::copy(path, &copy);
        // As `open_read` hands the file over to the window.
        let Ok(loaded) = crate::read_file(&copy) else { continue };
        checked += 1;
        let shown: gpui_kit::SharedString = name.clone().into();
        s.update(|e, window, cx| {
            e.load(loaded.presentation, shown, window, cx);
            e.set_loaded_source(loaded.source);
            e.set_file_path(Some(copy.clone()));
        });
        s.settle();
        // Under another name: a file with a password to modify refuses to be written over.
        let target = out.join(format!("app-saved-{name}"));
        let saved = s.update(|e, window, _| e.save_to(target.clone(), window));
        let outcome = match saved {
            Err(error) => Some(format!("save: {error}")),
            Ok(()) => {
                let after = std::fs::read(&target).unwrap_or_default();
                match ibis_formats::ibis_ooxml::canon::diff_packages(&original, &after, &[]) {
                    Ok(report) if report.is_same() => None,
                    Ok(report) => {
                        let names: Vec<String> = report.differences.iter().take(5).map(|(n, _)| n.clone()).collect();
                        Some(format!("{} part(s) differ: {}", report.differences.len(), names.join(" ")))
                    }
                    Err(error) => Some(format!("parts: {error:?}")),
                }
            }
        };
        let _ = std::fs::remove_file(&copy);
        let _ = std::fs::remove_file(&target);
        if let Some(detail) = outcome {
            failures.push((name, detail));
        }
    }
    let table: String = failures.iter().map(|(n, d)| format!("{n}\tfail\t{d}\n")).collect();
    let _ = std::fs::write(out.join("app-result.tsv"), format!("file\tstatus\tdetail\n{table}"));
    eprintln!("open and save: {checked} files checked, {} failed (target/roundtrip/app-result.tsv)", failures.len());
    assert!(checked >= 20 || std::env::var("IBIS_ROUNDTRIP_ONLY").is_ok(), "the public test files were found: {checked}");
    let new: Vec<String> = failures.iter().filter(|(n, _)| !KNOWN.iter().any(|(k, _)| k == n)).map(|(n, d)| format!("{n}: {d}")).collect();
    assert!(new.is_empty(), "files saved as other files without an edit:\n{}", new.join("\n"));
}


fn save_image_preview_keeps_transparent_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preview.tga");
    // A 1 by 1 uncompressed 32-bit TGA, with one semitransparent BGRA pixel.
    let mut tga = vec![0; 18];
    tga[2] = 2;
    tga[12] = 1;
    tga[14] = 1;
    tga[16] = 32;
    tga[17] = 0x28;
    tga.extend([30, 20, 10, 128]);
    std::fs::write(&path, &tga).unwrap();

    let mut s = Scenario::start_view();
    s.update(|e, w, cx| e.open_path(path.clone(), None, w, cx));
    for _ in 0..100 {
        s.wait(50);
        if s.with(|e, _| e.has_document && !e.presentation().media.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(s.with(|e, _| e.has_document && !e.presentation().media.is_empty()));
    let target = path.with_extension("docx");
    s.update(|e, w, _| e.save_to(target.clone(), w)).unwrap();

    let saved = ibis_formats::read(&std::fs::read(target).unwrap()).unwrap();
    let media = saved.media.last().expect("the saved DOCX contains the imported image");
    let pixel = image::load_from_memory(&media.data).unwrap().to_rgba8();
    assert_eq!(pixel.dimensions(), (1, 1));
    assert_eq!(pixel.get_pixel(0, 0).0, [10, 20, 30, 128]);
    assert_eq!(std::fs::read(path).unwrap(), tga);
}

fn save_reports_markdown_source_loss() {
    use crate::shell::actions::Save;
    use ibis_lawtext::{parse_law_xml, to_markdown};

    let xml = include_str!("../../../../ibis-lawtext/tests/lawtext/food-labeling-table.xml");
    let markdown = to_markdown(&parse_law_xml(xml).unwrap())
        .replace(" {#ibis_law_table_0}", "")
        .replace("| 形質 |", "| 形質区分 |");
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("law.md");
    std::fs::write(&input, markdown).unwrap();
    let mut scenario = Scenario::open(input.to_str().unwrap());
    let target = directory.path().join("law.xml");
    scenario.update(|editor, window, cx| {
        editor.set_file_path(Some(target.clone()));
        editor.save(&Save, window, cx);
    });
    scenario.settle();

    let expected = crate::shell::i18n::format(
        "file.markdown-source-unrestored",
        &[("count", &1), ("locations", &"別表第十八")],
    );
    let status = scenario.status_texts();
    assert!(status.contains(&expected), "save status: {status:?}");
    let output = std::fs::read_to_string(target).unwrap();
    assert!(output.contains("形質区分"));
    assert!(!output.contains("rowspan="));
    if std::env::var_os("IBIS_SAVE_DIAGNOSTIC_SCREENSHOT").is_some() {
        scenario.screenshot("law-source-loss-status");
    }
}

fn markdown_desktop_roundtrip() {
    use crate::docx::{ext::*, scenarios::DocSteps as _};
    use ibis_doc_model::{Block, FlowPos, FlowRunKind, Link};

    let input = crate::shell::scenario::repo_root().join("test/fixtures/markdown-desktop");
    let workspace = tempfile::tempdir().unwrap();
    let folder = workspace.path();
    std::fs::create_dir(folder.join("images")).unwrap();
    for name in ["index.md", "monthly-revenue.md", "briefing.md", "images/revenue.png"] {
        std::fs::copy(input.join(name), folder.join(name)).unwrap();
    }

    let mut doc = Scenario::start_view();
    doc.open_folder(folder.to_path_buf());
    doc.open_folder_document("monthly-revenue.md");
    if std::env::var_os("IBIS_MARKDOWN_DESKTOP_SCREENSHOTS").is_some() {
        doc.screenshot("okf-folder-and-document");
    }
    let first = match &doc.document().body[0] {
        Block::Paragraph(first) => first.clone(),
        _ => panic!("expected editable introductory paragraph"),
    };
    doc.click_text(FlowPos::new(first.id, first.len()));
    doc.type_text(" Revised.");
    let original = folder.join("monthly-revenue.md");
    doc.update(|editor, window, _| editor.save_to(original.clone(), window)).unwrap();
    let markdown = std::fs::read_to_string(&original).unwrap();
    assert!(markdown.contains("Monthly revenue review. Revised."), "{markdown}");
    assert!(markdown.contains("x-audit:\n  source: synthetic # keep this comment"), "{markdown}");
    assert!(markdown.contains("[Revenue figure](#fig_revenue)"), "{markdown}");
    assert!(markdown.contains("[Revenue equation](#eq_revenue)"), "{markdown}");
    assert!(markdown.contains("R=pq"), "{markdown}");
    assert!(!markdown.contains("![[images/revenue.png"), "{markdown}");
    doc.reload_saved();

    let saved = folder.join("monthly-revenue-edited.docx");
    doc.update(|editor, window, _| editor.save_to(saved.clone(), window)).unwrap();
    doc.reload_saved();

    let document = doc.document();
    let paragraphs = ibis_doc_model::flow_command::all_paragraphs(&document);
    assert!(paragraphs[0].text().contains("Revised."));
    for label in ["fig_revenue", "eq_revenue"] {
        let target = paragraphs.iter()
            .find(|paragraph| paragraph.bookmarks.iter().any(|name| name == label))
            .expect("saved bookmark").id;
        let anchor = Link::Anchor(label.into());
        let source = paragraphs.iter().find_map(|paragraph| {
            let mut offset = 0;
            for run in &paragraph.runs {
                if run.link.as_ref() == Some(&anchor) {
                    return Some(FlowPos::new(paragraph.id, offset));
                }
                offset += run.len();
            }
            None
        }).expect("saved reference");
        assert!(doc.update(|editor, _, cx| editor.follow_link(source, cx)));
        assert_eq!(doc.selection().unwrap().1, FlowPos::new(target, 0));
    }
    let pictures: Vec<_> = paragraphs.iter().flat_map(|paragraph| &paragraph.runs)
        .filter_map(|run| match &run.kind {
            FlowRunKind::Picture { size, .. } => Some(*size),
            _ => None,
        }).collect();
    assert_eq!(pictures.len(), 2);
    assert_eq!(pictures[1], ibis_doc_model::EmuSize::new(120 * 9_525, 60 * 9_525));

    let markdown = folder.join("monthly-revenue-edited.md");
    doc.update(|editor, window, _| editor.save_to(markdown.clone(), window)).unwrap();
    doc.reload_saved();
    let exported = std::fs::read_to_string(markdown).unwrap();
    assert!(exported.contains("verified: {by: \"human:reviewer\", at: \"2020-01-02\"}"));
    assert!(exported.contains("Revised."));
    assert!(doc.document().body.iter().any(|block| matches!(block, Block::Paragraph(para)
        if para.runs.iter().any(|run| run.link.as_ref() == Some(&Link::Anchor("eq_revenue".into()))))));

    drop(doc);
    let mut deck = Scenario::start_view();
    deck.open_folder(folder.to_path_buf());
    deck.open_folder_document("briefing.md");
    deck.double_click_shape(0);
    deck.type_text(" Revised");
    let original = folder.join("briefing.md");
    deck.update(|editor, window, _| editor.save_to(original.clone(), window)).unwrap();
    let markdown = std::fs::read_to_string(&original).unwrap();
    assert!(markdown.contains("x-ibis-format: presentation"), "{markdown}");
    assert!(markdown.contains("Revised"), "{markdown}");
    assert!(markdown.contains("monthly totals"), "{markdown}");
    deck.reload_saved();

    let saved = folder.join("briefing-edited.pptx");
    deck.update(|editor, window, _| editor.save_to(saved.clone(), window)).unwrap();
    deck.reload_saved();
    if std::env::var_os("IBIS_MARKDOWN_DESKTOP_SCREENSHOTS").is_some() {
        deck.update(|editor, window, cx| editor.show_slide_number(2, window, cx));
        deck.settle();
        deck.screenshot("marp-saved-slide");
    }
    let presentation = deck.with(|editor, _| editor.presentation().clone());
    let slide_text = |index: usize| {
        presentation.slides[index].shapes.iter().filter_map(|shape| shape.text_body())
            .flat_map(|body| &body.paragraphs).flat_map(|paragraph| &paragraph.runs)
            .map(|run| run.text.as_str()).collect::<Vec<_>>()
    };
    assert!(slide_text(0).join("").contains("Revised"));
    assert!(!slide_text(0).contains(&"1"));
    assert!(slide_text(1).contains(&"1"));
    assert!(slide_text(2).contains(&"1"));
    assert!(presentation.slides[1].notes.contains("monthly totals"));
}
