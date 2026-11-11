// SPDX-License-Identifier: AGPL-3.0-only
//! Exercise the actual native open path with a synthetic JTD, without writing its source.
use super::Scenario;
use crate::docx::scenarios::DocSteps;

pub(crate) fn all() -> Vec<(&'static str, fn())> {
    vec![("shell::native_jtd_open_read_only", open_read_only), ("shell::native_jtd_omissions", omissions), ("shell::native_jtd_template", template), ("shell::native_jtd_compressed_template", compressed_template)]
}

fn compressed_template() {
    let mut s = Scenario::start_view();
    let path = s.screenshot_path("synthetic-compressed-template").with_extension("JTTC");
    let bytes = include_bytes!("../../../../../test/fixtures/jtd/synthetic.jttc");
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(ibis_pptx::Kind::from_extension("JTTC"), Some(ibis_pptx::Kind::Document));
    assert!(ibis_pptx::Kind::OPENABLE_EXTENSIONS.contains(&"jttc"));
    s.update(|e, w, cx| e.open_path(path.clone(), None, w, cx));
    for _ in 0..100 {
        s.wait(50);
        if s.with(|e, _| e.is_jtd_read_only()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(s.with(|e, _| e.is_jtd_read_only()));
    assert!(s.paragraph(0).1.contains("日本語 test"));
    s.update(|e, w, cx| e.edit_jtd_as_docx(w, cx));
    assert!(!s.with(|e, _| e.is_jtd_read_only()));
    assert!(s.update(|e, w, _| e.save_to(path.clone(), w)).is_err());
    assert!(s.update(|e, w, _| e.save_to(path.with_file_name("other.jttc"), w)).is_err());
    assert!(s.update(|e, w, _| e.save_to(path.with_file_name("other.jtdc"), w)).is_err());
    let docx = path.with_extension("docx");
    s.update(|e, w, _| e.save_to(docx.clone(), w)).unwrap();
    assert!(ibis_pptx::read(&std::fs::read(docx).unwrap()).is_ok());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let renamed = path.with_file_name("renamed-compressed.docx");
    std::fs::write(&renamed, bytes).unwrap();
    assert!(crate::read_file(&renamed).unwrap().jtd_original.is_some());
    let invalid = path.with_file_name("invalid.jttc");
    std::fs::write(&invalid, b"not a JTTC").unwrap();
    assert!(crate::read_file(&invalid).is_err());
}

fn template() {
    let mut s = Scenario::start_view();
    let path = s.screenshot_path("synthetic-template").with_extension("JTT");
    let bytes = include_bytes!("../../../../../test/fixtures/jtd/synthetic.jtd");
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(ibis_pptx::Kind::from_extension("JTT"), Some(ibis_pptx::Kind::Document));
    assert!(ibis_pptx::Kind::OPENABLE_EXTENSIONS.contains(&"jtt"));
    s.update(|e, w, cx| e.open_path(path.clone(), None, w, cx));
    for _ in 0..100 {
        s.wait(50);
        if s.with(|e, _| e.is_jtd_read_only()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(s.with(|e, _| e.is_jtd_read_only()));
    assert!(s.paragraph(0).1.contains("日本語 test"));
    s.update(|e, w, cx| e.edit_jtd_as_docx(w, cx));
    assert!(!s.with(|e, _| e.is_jtd_read_only()));
    assert!(s.update(|e, w, _| e.save_to(path.clone(), w)).is_err());
    assert!(s.update(|e, w, _| e.save_to(path.with_file_name("other.jtt"), w)).is_err());
    let docx = path.with_extension("docx");
    s.update(|e, w, _| e.save_to(docx.clone(), w)).unwrap();
    assert!(ibis_pptx::read(&std::fs::read(docx).unwrap()).is_ok());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let invalid = path.with_file_name("invalid.jtt");
    std::fs::write(&invalid, b"not a JTT").unwrap();
    assert!(crate::read_file(&invalid).is_err());
}

fn omissions() {
    use ibis_ooxml::cfb::{self, Item};
    let original = include_bytes!("../../../../../test/fixtures/jtd/synthetic.jtd");
    let text = cfb::stream_path(original, "DocumentText").unwrap();
    let bytes = cfb::write(&[
        Item::storage("ObjectSheets"),
        Item::storage("ObjectSheets/DocSheet"),
        Item::storage("ObjectSheets/DocSheet/DOCS_0001"),
        Item::stream("ObjectSheets/DocSheet/DOCS_0001/DocumentText", text.clone()),
        Item::storage("Embedding 1"),
        Item::stream("DocumentText", text),
    ]);
    let mut s = Scenario::start_view();
    let path = s.screenshot_path("jtd-omissions").with_extension("jtd");
    std::fs::write(&path, bytes).unwrap();
    s.update(|e, w, cx| e.open_path(path, None, w, cx));
    for _ in 0..100 {
        s.wait(50);
        if s.with(|e, _| e.is_jtd_read_only()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    s.with(|e, _| {
        let status = e.status.as_deref().unwrap();
        assert!(status.contains(&crate::shell::i18n::format("jtd.omitted-sheets", &[("count", &1)])));
        assert!(status.contains(&crate::shell::i18n::format("jtd.omitted-objects", &[("count", &1)])));
    });
    s.update(|e, w, cx| e.edit_jtd_as_docx(w, cx));
    s.with(|e, _| {
        assert!(!e.is_jtd_read_only());
        let status = e.status.as_deref().unwrap();
        assert!(status.contains(&crate::shell::i18n::format("jtd.omitted-sheets", &[("count", &1)])));
        assert!(status.contains(&crate::shell::i18n::format("jtd.omitted-objects", &[("count", &1)])));
    });
}

fn open_read_only() {
    let mut s = Scenario::start_view();
    let path = s.screenshot_path("synthetic").with_extension("JTD");
    let bytes = include_bytes!("../../../../../test/fixtures/jtd/synthetic.jtd");
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(ibis_pptx::Kind::from_extension("JTD"), Some(ibis_pptx::Kind::Document));
    assert!(ibis_pptx::Kind::OPENABLE_EXTENSIONS.contains(&"jtd"));
    s.update(|e, w, cx| e.open_path(path.clone(), None, w, cx));
    for _ in 0..100 {
        s.wait(50);
        if s.with(|e, _| e.is_jtd_read_only()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(s.with(|e, _| e.is_read_only() && e.is_jtd_read_only()));
    assert!(s.paragraph(0).1.contains("日本語 test"));
    let (paragraph, _) = s.paragraph(0);
    s.click_text(ibis_doc_model::FlowPos::new(paragraph, 0));
    s.type_text("forbidden");
    assert!(!s.paragraph(0).1.contains("forbidden"));
    s.update(|e, w, cx| e.save(&crate::Save, w, cx));
    s.update(|e, w, cx| e.save_as(&crate::SaveAs, w, cx));
    assert!(s.update(|e, w, _| e.save_to(path.clone(), w)).is_err());
    assert!(s.with(|e, _| e.copy_to(&path)).is_err());
    assert!(s.with(|e, _| e.write_pdf(path.clone())).is_err());
    assert!(s.with(|e, _| e.write_png(path.clone(), None)).is_err());
    let output = path.with_extension("docx");
    assert!(s.with(|e, _| e.export_jtd_docx_to(&path)).is_err());
    #[cfg(unix)]
    {
        let alias = path.with_file_name("alias.docx");
        let _ = std::fs::remove_file(&alias);
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(s.with(|e, _| e.export_jtd_docx_to(&alias)).is_err());
    }
    s.with(|e, _| e.export_jtd_docx_to(&output)).unwrap();
    let imported = ibis_pptx::read(&std::fs::read(output).unwrap()).unwrap();
    assert!(imported.flow.unwrap().body.iter().any(|b| matches!(b, ibis_doc_model::Block::Paragraph(p) if p.text().contains("日本語 test"))));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    // Editing begins from a DOCX model, with no path to the original file.
    s.update(|e, w, cx| { e.read_only = true; e.edit_jtd_as_docx(w, cx); });
    assert!(s.with(|e, _| e.is_jtd_read_only()));
    s.update(|e, _, _| e.read_only = false);
    s.update(|e, w, cx| e.edit_jtd_as_docx(w, cx));
    assert!(s.with(|e, _| !e.is_read_only() && !e.is_jtd_read_only()));
    let (paragraph, _) = s.paragraph(0);
    s.click_text(ibis_doc_model::FlowPos::new(paragraph, 0));
    s.type_text("edited");
    assert!(s.paragraph(0).1.contains("edited"));
    assert!(s.update(|e, w, _| e.save_to(path.clone(), w)).is_err());
    assert!(s.with(|e, _| e.copy_to(&path)).is_err());
    assert!(s.with(|e, _| e.write_pdf(path.clone())).is_err());
    assert!(s.with(|e, _| e.write_png(path.clone(), None)).is_err());
    assert!(s.update(|e, w, _| e.save_to(path.with_file_name("other.jtd"), w)).is_err());
    #[cfg(unix)]
    {
        let alias = path.with_file_name("alias.docx");
        assert!(s.update(|e, w, _| e.save_to(alias, w)).is_err());
    }
    let edited_path = path.with_file_name("edited.docx");
    s.update(|e, w, _| e.save_to(edited_path.clone(), w)).unwrap();
    let edited = ibis_pptx::read(&std::fs::read(edited_path).unwrap()).unwrap();
    assert!(edited.flow.unwrap().body.iter().any(|b| matches!(b, ibis_doc_model::Block::Paragraph(p) if p.text().contains("edited"))));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    // The signature, not just the extension, keeps a renamed JTD read-only.
    let renamed = path.with_extension("docx");
    std::fs::write(&renamed, bytes).unwrap();
    let loaded = crate::read_file(&renamed).unwrap();
    assert!(loaded.jtd_original.is_some());
    std::fs::write(&renamed, b"not a JTD").unwrap();
    let invalid = path.with_file_name("invalid.jtd");
    std::fs::write(&invalid, b"not a JTD").unwrap();
    assert!(crate::read_file(&invalid).is_err());
}
