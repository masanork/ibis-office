// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! GitHub issue #235: a PDF is opened for viewing (pure Rust: `ibis-pdf` reads it, `ibis-office-pdf` draws its pages), as a screen of its own.
use crate::shell::scenario::Scenario;

pub(crate) fn all() -> Vec<(&'static str, fn())> {
    let mut scenarios: Vec<(&str, fn())> = vec![
        ("shell::issue235_a_pdf_opens_as_a_screen_of_its_own_with_its_pages", opens),
        ("shell::issue235_the_menus_of_a_pdf_are_those_of_a_file_that_is_looked_at", menus),
        ("shell::issue235_a_copy_and_an_export_of_a_pdf_are_the_file_as_it_is", copies),
        ("shell::issue235_text_is_picked_by_dragging_and_copied", pick_and_copy),
        ("shell::issue235_all_the_text_is_selected_and_copied_page_by_page", select_all_and_copy),
        ("shell::issue235_the_text_is_searched_on_all_the_pages", search),
        ("shell::issue235_a_page_is_gone_to_by_its_number", go_to_page),
        ("shell::vsdx_search_highlights_a_wrapped_label", vsdx_search),
        ("shell::vsdx_opens_a_diagram_with_embedded_images", vsdx_embedded_images),
        ("shell::vsdx_new_diagram_saves_and_reopens_glued_shapes", vsdx_new_diagram),
        ("shell::vsdx_existing_diagram_updates_a_label_in_a_copy", vsdx_edit_label),
        ("shell::vsdx_label_edits_are_kept_when_switching_pages", vsdx_edit_multiple_pages),
        ("shell::vsdx_existing_geometry_survives_reopening", vsdx_edit_geometry),
    ];
    if std::env::var_os("VSDX_VISIO_GLUED_ARC_SAMPLE").is_some() {
        scenarios.push(("shell::vsdx_visio_authored_drawing_edit_mode", vsdx_visio_authored_edit_mode));
        scenarios.push(("shell::vsdx_visio_authored_group_drag", vsdx_visio_authored_group_drag));
    }
    if std::env::var_os("VSDX_VISIO_TRANSFORMED_GROUP_SAMPLE").is_some() {
        scenarios.push(("shell::vsdx_visio_transformed_group_drag", vsdx_visio_transformed_group_drag));
    }
    scenarios
}

fn opens() {
    let mut s = Scenario::open("test/editing/mac-office-shape.pdf");
    assert_eq!(s.with(|e, _| e.kind()), ibis_formats::Kind::Pdf);
    let pages = s.with(|e, _| e.presentation().slides.len());
    assert!(pages >= 1, "the file's pages are the screen's pages: {pages}");
    assert!(s.with(|e, _| e.presentation().pdf_bytes.is_some()));
    // The file's own name and kind are shown, and the page count is in the status bar.
    let texts = s.status_texts();
    assert!(!texts.is_empty(), "the status bar says something: {texts:?}");
    s.screenshot("pdf-issue235-opened");
}

/// The menu bar and the toolbar are the document screen's, whole: what a PDF cannot do is grey, not left out.
fn menus() {
    use crate::shell::chrome::{MenuEntry, MenuId, ToolItem};
    fn enabled(entries: &[MenuEntry], out: &mut Vec<&'static str>, all: &mut usize) {
        for entry in entries {
            match entry {
                MenuEntry::Item(item) => {
                    *all += 1;
                    if !item.disabled && item.handler.is_some() {
                        out.push(item.id);
                    }
                }
                MenuEntry::Submenu(sub) => enabled(&sub.items, out, all),
                _ => {}
            }
        }
    }
    let mut s = Scenario::open("test/editing/mac-office-shape.pdf");
    let (ids, on, all, buttons) = s.with(|e, _| {
        let ids: Vec<MenuId> = crate::shell::menu_layout::layout(ibis_formats::Kind::Pdf).iter().map(|m| m.id).collect();
        let bar = e.menu_bar_menus().menus;
        let (mut on, mut all) = (Vec::new(), 0);
        for (_, entries) in &bar {
            enabled(entries, &mut on, &mut all);
        }
        let buttons: Vec<(&'static str, bool)> = e.toolbar_items().iter().filter_map(|t| if let ToolItem::Button(b) = t { Some((b.id, b.disabled || b.handler.is_none())) } else { None }).collect();
        (ids, on, all, buttons)
    });
    // The document screen's menus, whole.
    assert_eq!(ids.len(), 11, "{ids:?}");
    for menu in [MenuId::Insert, MenuId::Layout, MenuId::Review] {
        assert!(ids.contains(&menu), "{menu:?} stays, grey");
    }
    assert!(all > 150, "the document's lines are all there: {all}");
    // What works: opening, closing, a copy, the PDF written out, printing, the other application, the zoom, the windows, the help.
    for id in ["file.open", "file.close", "file.copy", "file.export.pdf", "file.print", "window.open-in-office", "view.zoom-in", "view.zoom-fit", "window.list", "help.about"] {
        assert!(on.contains(&id), "{id} works: {on:?}");
    }
    // What does not is grey, not left out: the document's own commands, saving, the pictures of the pages.
    for id in ["file.save", "file.save-as", "file.export.png", "file.export.gif", "file.protect"] {
        assert!(!on.contains(&id), "{id} is grey");
    }
    assert!(on.iter().all(|id| !id.starts_with("docx.") || matches!(*id, "docx.never")), "a document's command works on a PDF: {on:?}");
    // A PDF with no text (this one is pictures and shapes) has nothing to pick or search: Select all and Find are grey; Go to a page works.
    assert!(!on.contains(&"edit.select-all") && !on.contains(&"edit.find-replace"), "no text: Select all and Find are grey: {on:?}");
    assert!(on.contains(&"edit.go-to"), "a page is gone to without any text: {on:?}");
    let working: Vec<&str> = buttons.iter().filter(|b| !b.1).map(|b| b.0).collect();
    assert!(!working.contains(&"formatting-marks"), "the document's buttons are grey: {working:?}");
    eprintln!("enabled buttons: {working:?}");
    use crate::shell::format::FormatEditor;
    let info = ibis_office_pdf::editor::PdfDocument::INFO;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let open_in = info.office.open_key;
        assert!(open_in.starts_with("window.open-pdf-in-"), "the PDF's own words, not Word's: {open_in}");
    }
    assert_eq!(info.save_formats.len(), 0, "a PDF is not saved as another kind");
    s.screenshot("pdf-issue235-menus");
    // (One screen at a time.)
    drop(s);
    // One with text (a file of the scenarios below): they work.
    let dir = crate::shell::scenario::repo_root().join("target/scenario/issue235-menus-src");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("text.pdf"), text_pdf()).unwrap();
    let mut with_text = Scenario::open("target/scenario/issue235-menus-src/text.pdf");
    let on_text = with_text.with(|e, _| {
        let (mut on, mut all) = (Vec::new(), 0);
        for (_, entries) in &e.menu_bar_menus().menus {
            enabled(entries, &mut on, &mut all);
        }
        on
    });
    assert!(on_text.contains(&"edit.select-all") && on_text.contains(&"edit.find-replace") && on_text.contains(&"edit.go-to"), "text: they work: {on_text:?}");
}

/// A copy saved, and the PDF written by 書き出し, are the bytes of the file that was opened.
fn copies() {
    let mut s = Scenario::open("test/editing/mac-office-shape.pdf");
    let original = std::fs::read(crate::shell::scenario::repo_root().join("test/editing/mac-office-shape.pdf")).unwrap();
    let dir = crate::shell::scenario::repo_root().join("target/scenario/issue235-copies");
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("copy.pdf");
    s.with(|e, _| e.copy_to(&copy)).expect("a copy");
    assert_eq!(std::fs::read(&copy).unwrap(), original);
    let out = dir.join("export.pdf");
    let job = s.with(|e, _| e.write_pdf(out.clone())).expect("a job");
    job().expect("written");
    assert_eq!(std::fs::read(&out).unwrap(), original);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A PDF of two pages with text in a standard font (what the scenarios of the text work on): the first has two lines, the second one.
fn text_pdf() -> String {
    let pages = ["BT /F1 24 Tf 72 720 Td (Hello PDF world) Tj ET\nBT /F1 24 Tf 72 680 Td (Search the Text here) Tj ET", "BT /F1 24 Tf 72 720 Td (Second page text) Tj ET"];
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 7 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", pages[0].len(), pages[0]),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", pages[1].len(), pages[1]),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.drain(..).enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1));
    for o in &offsets {
        out.push_str(&format!("{o:010} 00000 n \n"));
    }
    out.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1));
    out
}

fn open_text_pdf() -> Scenario {
    let dir = crate::shell::scenario::repo_root().join("target/scenario/issue235-text-src");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("text.pdf"), text_pdf()).unwrap();
    Scenario::open("target/scenario/issue235-text-src/text.pdf")
}

fn clipboard(s: &mut Scenario) -> Option<String> {
    s.with_app(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

fn position(s: &mut Scenario, page: usize, index: usize) -> (f32, f32) {
    s.with(|e, _| ibis_office_pdf::editor::char_position(e, page, index)).expect("the page was drawn")
}

/// Pressing on a character and dragging to another picks the text between them; the copy puts it on the clipboard; a click picks nothing.
fn pick_and_copy() {
    let mut s = open_text_pdf();
    assert_eq!(s.with(|e, _| e.kind()), ibis_formats::Kind::Pdf);
    // Nothing is picked: the copy is grey.
    assert_eq!(s.menu_state("edit.copy").map(|m| m.0), Some(false), "nothing picked: copy is grey");
    let (a, b) = (position(&mut s, 0, 0), position(&mut s, 0, 4));
    s.drag(a, b);
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::selected_text(e)), "Hello");
    s.menu("edit.copy");
    assert_eq!(clipboard(&mut s).as_deref(), Some("Hello"));
    // Dragging down to the second line picks to the end of the first and the start of the second.
    let (a, c) = (position(&mut s, 0, 6), position(&mut s, 0, 18));
    s.drag(a, c);
    let picked = s.with(|e, _| ibis_office_pdf::editor::selected_text(e));
    assert!(picked.starts_with("PDF world\nSear"), "{picked:?}");
    // A click without a drag picks nothing.
    s.click(a);
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::selected_text(e)), "");
    s.screenshot("pdf-issue235-selection");
}

/// 編集 > すべて選択 picks the text of every page; the copy has a line break between lines and a blank line between pages.
fn select_all_and_copy() {
    let mut s = open_text_pdf();
    s.menu("edit.select-all");
    s.menu("edit.copy");
    assert_eq!(clipboard(&mut s).as_deref(), Some("Hello PDF world\nSearch the Text here\n\nSecond page text"));
}

/// ⌘F opens the search bar; what is typed is looked for on all the pages without regard to case; Enter goes to the next place (and shows its
/// page), Shift+Enter to the previous one.
fn search() {
    let mut s = open_text_pdf();
    // The pointer is on the page (as when a person reads it): the keys go to the screen.
    let p = position(&mut s, 0, 3);
    s.click(p);
    s.key("cmd-f");
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::find_status(e)), Some((0, None)), "the bar is open");
    s.type_text("text");
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::find_status(e)), Some((2, None)), "two places, on two pages");
    s.key("enter");
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::find_status(e)), Some((2, Some(0))));
    s.key("enter");
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::find_status(e)), Some((2, Some(1))));
    assert!(s.status_texts()[0].contains("2 / 2"), "the second place is on the second page: {:?}", s.status_texts());
    s.key("shift-enter");
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::find_status(e)), Some((2, Some(0))));
    assert!(s.status_texts()[0].contains("1 / 2"), "{:?}", s.status_texts());
    s.screenshot("pdf-issue235-search");
    // Nothing found.
    s.type_text("zzz");
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::find_status(e)), Some((0, None)));
}

/// ジャンプ… takes the number of a page: the page is shown and the status bar says it.
fn go_to_page() {
    let mut s = open_text_pdf();
    assert!(s.status_texts()[0].contains("1 / 2"), "{:?}", s.status_texts());
    let p = position(&mut s, 0, 3);
    s.click(p);
    s.menu("edit.go-to");
    s.type_text("2");
    s.key("enter");
    assert!(s.status_texts()[0].contains("2 / 2"), "{:?}", s.status_texts());
    // The page before it, by the page keys.
    s.key("pageup");
    assert!(s.status_texts()[0].contains("1 / 2"), "{:?}", s.status_texts());
}

fn vsdx_search() {
    let mut s = Scenario::open("crates/ibis-vsdx/tests/fixtures/07-zh-flowchart.vsdx");
    assert!(s.with(|e, _| e.is_read_only()));
    let p = position(&mut s, 0, 0);
    s.click(p);
    s.key("cmd-f");
    s.type_text("转正答辩通过？");
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::find_status(e)), Some((1, None)));
    s.key("enter");
    assert_eq!(s.with(|e, _| ibis_office_pdf::editor::find_status(e)), Some((1, Some(0))));
    s.screenshot("vsdx-cjk-search");
}

fn vsdx_embedded_images() {
    let mut s = Scenario::open("crates/ibis-vsdx/tests/fixtures/09-embedded-media-icons.vsdx");
    assert!(s.with(|e, _| e.is_read_only()));
    assert_eq!(s.with(|e, _| e.presentation().slides.len()), 1);
    s.screenshot("vsdx-embedded-images");
}

fn vsdx_new_diagram() {
    use super::button_probe as buttons;
    let _probe = buttons::Probe::new();
    let mut s = Scenario::start_view();
    let x = |id| buttons::bounds(id).unwrap().center().x;
    assert!(x("start-new-pptx") < x("start-new-vsdx"));
    assert!(x("start-new-vsdx") < x("start-new-md"));
    buttons::click(&mut s, "start-new-vsdx");
    assert_eq!(s.with(|editor, _| editor.kind()), ibis_formats::Kind::Presentation);
    assert!(s.with(|editor, _| editor.slide().shapes.is_empty()));
    let point = |s: &mut Scenario, x, y| {
        s.with(|e, _| {
            let layout = e.slide_layout().unwrap();
            (f32::from(layout.origin.x + layout.to_px(x)), f32::from(layout.origin.y + layout.to_px(y)))
        })
    };
    for (tool, x0, x1) in [("shape.rect", 1_000_000, 2_500_000), ("shape.ellipse", 5_000_000, 7_000_000)] {
        s.toolbar_menu("insert-shape", tool);
        let a = point(&mut s, x0, 2_000_000);
        let b = point(&mut s, x1, 3_500_000);
        s.drag(a, b);
    }
    s.toolbar_menu("insert-line", "pptx.arrow");
    let a = point(&mut s, 2_500_000, 2_750_000);
    let b = point(&mut s, 5_000_000, 2_750_000);
    s.drag(a, b);
    assert_eq!(s.with(|editor, _| editor.slide().shape_glues.len()), 1);
    assert!(s.with(|editor, _| editor.name_parts().1 == "vsdx"));
    let dir = crate::shell::scenario::repo_root().join("target/scenario/vsdx-new-save-src");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("saved-diagram.vsdx");
    s.update(|editor, window, _| editor.save_to(path.clone(), window)).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(s.with(|editor, _| editor.file_path()), Some(path.clone()));
    assert!(!s.with(|editor, _| editor.is_dirty()));

    // The next regular Save must keep writing VSDX to the same file.
    s.toolbar_menu("insert-shape", "shape.rect");
    let a = point(&mut s, 1_000_000, 4_000_000);
    let b = point(&mut s, 2_500_000, 5_000_000);
    s.drag(a, b);
    assert!(s.with(|editor, _| editor.is_dirty()));
    s.key("cmd-s");
    let bytes = std::fs::read(&path).unwrap();
    assert_ne!(bytes, before);
    assert!(!s.with(|editor, _| editor.is_dirty()));
    let zip = ibis_formats::zip::Archive::new(&bytes).unwrap();
    let xml = zip.read_bounded("visio/pages/page1.xml", 100_000).unwrap().unwrap();
    let xml = String::from_utf8(xml).unwrap();
    assert_eq!(xml.matches("<Shape ID=").count(), 4);
    assert!(xml.contains(r#"FromSheet="4" FromCell="BeginX" FromPart="9" ToSheet="1" ToCell="Connections.X2""#));
    assert!(xml.contains(r#"FromSheet="4" FromCell="EndX" FromPart="12" ToSheet="2" ToCell="Connections.X1""#));
    assert!(xml.contains(r#"N="EndArrow" V="4""#));
    drop(s);
    let mut reopened = Scenario::open("target/scenario/vsdx-new-save-src/saved-diagram.vsdx");
    assert!(reopened.with(|editor, _| editor.is_read_only()));
    assert_eq!(reopened.with(|editor, _| editor.presentation().slides.len()), 1);
    reopened.menu("vsdx.resume-edit");
    assert!(!reopened.with(|e, _| e.is_read_only()));
    let (original_x, start, end, connector) = reopened.with(|e, _| {
        let slide = e.slide();
        let (&connector, glue) = slide.shape_glues.iter().next().unwrap();
        assert_eq!(glue.start.unwrap().0, slide.shapes[0].id.0);
        assert_eq!(glue.end.unwrap().0, slide.shapes[1].id.0);
        (slide.shapes[0].frame.x, slide.shapes[0].id, slide.shapes[2].id, connector)
    });
    reopened.click_shape(0);
    reopened.key("escape");
    reopened.key("right");
    let moved_x = reopened.with(|e, _| e.slide().shapes[0].frame.x);
    assert!(moved_x > original_x);
    let (old_end, new_end) = reopened.with(|e, _| {
        let ellipse = e.slide().shapes[1].frame;
        let rect = e.slide().shapes[2].frame;
        ((ellipse.x, ellipse.y + ellipse.height / 2), (rect.x, rect.y + rect.height / 2))
    });
    reopened.click_shape(3);
    let from = point(&mut reopened, old_end.0, old_end.1);
    let to = point(&mut reopened, new_end.0, new_end.1);
    reopened.drag(from, to);
    reopened.with(|e, _| {
        let glue = e.slide().shape_glues[&connector];
        assert_eq!(glue.start.unwrap().0, start.0);
        assert_eq!(glue.end.unwrap().0, end.0);
    });
    reopened.key("cmd-s");
    assert!(!reopened.with(|e, _| e.is_dirty()));
    let saved = reopened.with(|e, _| e.file_path().unwrap());
    let new = std::fs::read(saved).unwrap();
    std::fs::write(dir.join("saved-after-reopen.vsdx"), new).unwrap();
    drop(reopened);

    let mut again = Scenario::open("target/scenario/vsdx-new-save-src/saved-after-reopen.vsdx");
    assert!(again.with(|e, _| e.is_read_only()));
    again.menu("vsdx.resume-edit");
    again.with(|e, _| {
        let slide = e.slide();
        assert!((slide.shapes[0].frame.x - moved_x).abs() <= 1);
        let glue = slide.shape_glues.values().next().unwrap();
        assert_eq!(glue.start.unwrap().0, slide.shapes[0].id.0);
        assert_eq!(glue.end.unwrap().0, slide.shapes[2].id.0);
    });
    again.screenshot("visio-saved-reopened");
}

fn vsdx_edit_label() {
    let mut s = Scenario::open("crates/ibis-vsdx/tests/fixtures/01-basic-flowchart.vsdx");
    let path = s.with(|editor, _| editor.file_path().unwrap());
    let before = std::fs::read(&path).unwrap();
    s.menu("vsdx.edit-label");
    s.key("cmd-a");
    s.type_text("更新 <Vista>");
    let bytes = s.with(|editor, cx| ibis_office_pdf::editor::label_copy_bytes(editor, cx)).unwrap();
    let zip = ibis_formats::zip::Archive::new(&bytes).unwrap();
    let page = zip.read_bounded("visio/pages/page1.xml", 100_000).unwrap().unwrap();
    assert!(String::from_utf8(page).unwrap().contains("更新 &lt;Vista&gt;"));
    let output = s.screenshot_path("updated-copy").with_extension("vsdx");
    std::fs::write(output, bytes).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), before, "the original file is untouched");
    s.screenshot("vsdx-label-edit");
}

fn vsdx_edit_multiple_pages() {
    let mut s = Scenario::open("crates/ibis-vsdx/tests/fixtures/08-multi-page-handbook.vsdx");
    s.menu("vsdx.edit-label");
    s.key("cmd-a");
    s.type_text("First page update");
    s.menu("edit.go-to");
    s.type_text("2");
    s.key("enter");
    assert!(s.status_texts()[0].contains("2 /"), "{:?}", s.status_texts());
    s.menu("vsdx.edit-label");
    s.key("cmd-a");
    s.type_text("Second page update");
    let bytes = s.with(|editor, cx| ibis_office_pdf::editor::label_copy_bytes(editor, cx)).unwrap();
    let zip = ibis_formats::zip::Archive::new(&bytes).unwrap();
    for (file, label) in [("page1.xml", "First page update"), ("page2.xml", "Second page update")] {
        let part = zip.read_bounded(&format!("visio/pages/{file}"), 100_000).unwrap().unwrap();
        assert!(String::from_utf8(part).unwrap().contains(label), "{file}");
    }
}

fn vsdx_edit_geometry() {
    geometry_mode_roundtrip(
        "crates/ibis-vsdx/tests/fixtures/01-basic-flowchart.vsdx",
        "move 2 0.25 0",
        "connect 14 end 6",
        r#"N="PinX" V="1.322835""#,
        r#"FromSheet="14" FromCell="EndX" ToSheet="6""#,
        "reconnected.vsdx",
        &["2", "12", "14"],
    );
}

fn vsdx_visio_authored_edit_mode() {
    let path = std::env::var("VSDX_VISIO_GLUED_ARC_SAMPLE").unwrap();
    geometry_mode_roundtrip(
        &path, "move 250 0.25 0", "connect 614 end 253",
        "",
        "FromSheet='614' FromCell='EndX' FromPart='12' ToSheet='253'",
        "visio-native-edited.vsdx",
        &["250", "614"],
    );
}

fn vsdx_visio_authored_group_drag() {
    let input = std::env::var("VSDX_VISIO_GLUED_ARC_SAMPLE").unwrap();
    group_drag_roundtrip(
        &input, "visio-group-child-edited.vsdx", ("536", "537"),
        (16.035433, 0.65), (1.7143403, 9.852173),
        ([0.280673746, 0.780542173], [0.530673746, 0.905542173]),
    );
}

fn vsdx_visio_transformed_group_drag() {
    let input = std::env::var("VSDX_VISIO_TRANSFORMED_GROUP_SAMPLE").unwrap();
    // Mario Pipes, Page-1. Group 48 is rotated 45 degrees and flipped vertically.
    let pin = 0.1377952755905518;
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    group_drag_roundtrip(
        &input, "visio-transformed-group-child-edited.vsdx", ("48", "50"),
        (45.271654, 0.40), (15.7672, 28.985147),
        ([pin, pin], [pin + 0.375 * diagonal, pin + 0.125 * diagonal]),
    );
}

fn group_drag_roundtrip(
    input: &str,
    filename: &str,
    (parent_id, child_id): (&str, &str),
    (height, zoom): (f32, f32),
    (x, y): (f32, f32),
    (original_pins, moved_pins): ([f64; 2], [f64; 2]),
) {
    use ibis_office_pdf::editor::{geometry_screen_point, staged_geometry_bytes, write_vsdx_copy};

    let mut s = Scenario::open(input);
    let path = s.with(|editor, _| editor.file_path().unwrap());
    let original = std::fs::read(&path).unwrap();
    s.menu("vsdx.resume-edit");
    s.size(1500.0, 1200.0);
    s.update(|editor, _, cx| {
        editor.zoom = Some(zoom);
        cx.notify();
    });
    // Move the center right 0.25 inches and up 0.125 inches in page coordinates.
    let mut screen = |x: f32, y: f32| {
        s.with(|editor, _| {
            geometry_screen_point(editor, 0, x * 72.0, (height - y) * 72.0).unwrap()
        })
    };
    let start = screen(x, y);
    let end = screen(x + 0.25, y + 0.125);
    s.drag(start, end);
    let edited = s.with(|editor, _| staged_geometry_bytes(editor))
        .unwrap_or_else(|| panic!("group drag failed: {:?}", s.status_texts()));

    let output = format!("target/scenario/vsdx-geometry-src/{filename}");
    let copy = crate::shell::scenario::repo_root().join(&output);
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    write_vsdx_copy(Some(&path), &copy, &edited).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), original);
    drop(s);
    let reopened = Scenario::open(&output)
        .with(|editor, _| editor.presentation().pdf_bytes.clone()).unwrap();
    let before = ibis_formats::zip::Archive::new(&original).unwrap();
    let after = ibis_formats::zip::Archive::new(&reopened).unwrap();
    assert_eq!(before.names().len(), after.names().len());
    for name in before.names() {
        let old = before.read(&name).unwrap();
        let new = after.read(&name).unwrap();
        if name != "visio/pages/page1.xml" {
            assert_eq!(old, new, "{name}");
            continue;
        }
        let old = String::from_utf8(old.unwrap()).unwrap();
        let new = String::from_utf8(new.unwrap()).unwrap();
        let redact_pins = |source: &str, expected: [f64; 2]| {
            let doc = roxmltree::Document::parse(source).unwrap();
            let shape = doc.descendants()
                .find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some(child_id))
                .unwrap();
            let parent = shape.parent().unwrap().parent().unwrap();
            assert_eq!(parent.attribute("ID"), Some(parent_id));
            let mut ranges = Vec::new();
            for ((pin, dimension), expected) in [("PinX", "Width"), ("PinY", "Height")].into_iter().zip(expected) {
                let pin = shape.children().find(|n| n.attribute("N") == Some(pin)).unwrap();
                let pos: f64 = pin.attribute("V").unwrap().parse().unwrap();
                let extent: f64 = parent.children().find(|n| n.attribute("N") == Some(dimension))
                    .unwrap().attribute("V").unwrap().parse().unwrap();
                let formula = pin.attribute("F").unwrap();
                let fraction: f64 = formula.strip_prefix(&format!("Sheet.{parent_id}!{dimension}*"))
                    .unwrap().parse().unwrap();
                assert!((pos - expected).abs() < 0.01, "{pin:?}: {pos}");
                assert!((pos - extent * fraction).abs() < 1e-9, "{formula}");
                ranges.push(pin.range());
            }
            let mut unchanged = source.to_owned();
            ranges.sort_unstable_by_key(|r| std::cmp::Reverse(r.start));
            for range in ranges {
                unchanged.replace_range(range, "");
            }
            unchanged
        };
        assert!(redact_pins(&old, original_pins)
            == redact_pins(&new, moved_pins),
            "only the child position cells should differ");
    }
}

fn geometry_mode_roundtrip(input: &str, move_command: &str, reconnect_command: &str, pin: &str, link: &str, filename: &str, edited_shapes: &[&str]) {
    use super::button_probe as buttons;
    use ibis_office_pdf::editor::{staged_geometry_bytes, write_vsdx_copy};
    let _probe = buttons::Probe::new();
    let mut s = Scenario::open(input);
    assert!(s.with(|editor, _| editor.is_read_only()));
    let path = s.with(|editor, _| editor.file_path().unwrap());
    let original = std::fs::read(&path).unwrap();
    s.menu("vsdx.resume-edit");
    if pin.is_empty() {
        s.size(1500.0, 1200.0);
        s.update(|editor, _, cx| { editor.zoom = Some(0.65); cx.notify(); });
        let screen = |s: &mut Scenario, x: f32, y: f32| {
            s.with(|editor, _| {
                let h = 16.035433_f32 * 72.0;
                ibis_office_pdf::editor::geometry_screen_point(editor, 0, x * 72.0, h - y * 72.0).unwrap()
            })
        };
        let from = screen(&mut s, 9.016714, 14.057004);
        let to = screen(&mut s, 9.266714, 14.057004);
        s.drag(from, to);
        assert!(s.with(|editor, _| staged_geometry_bytes(editor).is_some()), "{:?}", s.status_texts());
        let from = screen(&mut s, 6.612055, 3.960532);
        let to = screen(&mut s, 9.596682, 4.136813);
        s.drag(from, to);
    } else {
        s.type_text(move_command);
        buttons::click(&mut s, "vsdx-geometry-apply");
        assert!(s.with(|editor, _| staged_geometry_bytes(editor).is_some()), "{:?}", s.status_texts());
        s.type_text(reconnect_command);
        buttons::click(&mut s, "vsdx-geometry-apply");
    }
    let updated = s.with(|editor, _| staged_geometry_bytes(editor)).unwrap();
    let dir = crate::shell::scenario::repo_root().join("target/scenario/vsdx-geometry-src");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(write_vsdx_copy(Some(&path), &path, &updated).is_err());
    write_vsdx_copy(Some(&path), &dir.join(filename), &updated).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), original);
    drop(s);
    let mut final_view = Scenario::open(&format!("target/scenario/vsdx-geometry-src/{filename}"));
    let reopened = final_view.with(|editor, _| editor.presentation().pdf_bytes.clone()).unwrap();
    let before = ibis_formats::zip::Archive::new(&original).unwrap();
    let after = ibis_formats::zip::Archive::new(&reopened).unwrap();
    assert_eq!(before.names().len(), after.names().len());
    for name in before.names() {
        let original_part = before.read(&name).unwrap();
        let new_part = after.read(&name).unwrap();
        if name == "visio/pages/page1.xml" {
            let xml = String::from_utf8(new_part.unwrap()).unwrap();
            if pin.is_empty() {
                let doc = roxmltree::Document::parse(&xml).unwrap();
                let position = doc.descendants().find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some("250"))
                    .unwrap().children().find(|n| n.attribute("N") == Some("PinX")).unwrap()
                    .attribute("V").unwrap().parse::<f64>().unwrap();
                assert!((position - 9.266713647729857).abs() < 0.01, "{position}");
            } else {
                assert!(xml.contains(pin), "missing moved pin");
            }
            assert!(xml.contains(link), "missing connection");
            let shape_parts = |source: &str| {
                let doc = roxmltree::Document::parse(source).unwrap();
                doc.descendants().filter(|node| node.has_tag_name("Shape"))
                    .map(|node| (node.attribute("ID").unwrap().to_owned(), source[node.range()].to_owned()))
                    .collect::<std::collections::BTreeMap<_, _>>()
            };
            let old = String::from_utf8(original_part.unwrap()).unwrap();
            let original_shapes = shape_parts(&old);
            let new_shapes = shape_parts(&xml);
            assert_eq!(original_shapes.len(), new_shapes.len());
            let changed: Vec<_> = original_shapes.iter().filter_map(|(id, shape)| {
                (new_shapes.get(id) != Some(shape)).then_some(id.as_str())
            }).collect();
            let mut expected = edited_shapes.to_vec();
            expected.sort_unstable();
            assert_eq!(changed, expected);
        } else {
            assert_eq!(original_part, new_part, "{name}");
        }
    }
    final_view.screenshot("vsdx-reconnected");
}
