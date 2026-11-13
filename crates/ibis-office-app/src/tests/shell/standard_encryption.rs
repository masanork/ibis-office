// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
use super::Scenario;

pub(crate) fn all() -> Vec<(&'static str, fn())> {
    vec![("shell::standard_encryption_read_only", standard_encryption_read_only)]
}

fn standard_encryption_read_only() {
    let original = include_bytes!("../../../../ibis-ooxml/tests/fixtures/standard-password.docx");
    let mut s = Scenario::start_view();
    let path = s.screenshot_path("standard-original").with_extension("docx");
    let copy = s.screenshot_path("standard-copy").with_extension("docx");
    std::fs::write(&path, original).unwrap();
    let read = path.clone();
    s.update(|e, w, cx| e.open_read(path.clone(), move || crate::read_file_with(&read, Some("Password1234_")), None, w, cx));
    for _ in 0..200 {
        s.wait(50);
        if s.with(|e, _| e.standard_read_only) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(s.with(|e, _| e.standard_read_only && e.is_read_only()));
    assert!(s.update(|e, w, _| e.save_to(path.clone(), w)).is_err());
    assert!(s.with(|e, _| e.copy_to(&path)).is_err());
    let plaintext = path.with_extension("txt");
    assert!(s.update(|e, w, _| e.save_to(plaintext.clone(), w)).is_err());
    assert!(s.with(|e, _| e.copy_to(&plaintext)).is_err());
    assert!(!plaintext.exists());
    assert!(s.with(|e, _| e.check_standard_output(&path)).is_err());
    assert!(s.with(|e, _| e.write_pdf(path.clone())).is_err());
    assert!(s.with(|e, _| e.write_png(path.clone(), None)).is_err());
    assert!(s.with(|e, _| e.write_gif(path.clone(), None)).is_err());
    let hard_link = path.with_file_name("standard-hard-link.docx");
    let _ = std::fs::remove_file(&hard_link);
    std::fs::hard_link(&path, &hard_link).unwrap();
    assert!(s.with(|e, _| e.check_standard_output(&hard_link)).is_err());
    assert!(s.with(|e, _| e.copy_to(&hard_link)).is_err());
    std::fs::remove_file(hard_link).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    s.update(|e, w, _| e.save_to(copy.clone(), w)).unwrap();
    assert!(s.with(|e, _| !e.standard_read_only && !e.is_read_only()));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(s.update(|e, w, _| e.save_to(path.clone(), w)).is_err());
    let saved = std::fs::read(&copy).unwrap();
    assert!(ibis_pptx::is_encrypted(&saved));
    assert!(!ibis_pptx::ibis_ooxml::encryption::is_standard_encrypted(&saved));
    assert!(ibis_pptx::decrypt(&saved, "Password1234_").is_ok());
}
