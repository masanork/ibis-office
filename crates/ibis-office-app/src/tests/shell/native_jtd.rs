// SPDX-License-Identifier: AGPL-3.0-only
//! Exercise the actual native open path with a synthetic JTD, without writing its source.
use super::Scenario;
use crate::docx::scenarios::DocSteps;

pub(crate) fn all() -> Vec<(&'static str, fn())> {
    vec![("shell::native_jtd_open_read_only", open_read_only)]
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
