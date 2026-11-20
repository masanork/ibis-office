// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Exercise the native asynchronous open path, rather than Scenario::open's
//! shortcut, for signed and malformed packages.

use super::{repo_root, Scenario};
use ibis_formats::ibis_ooxml::Package;

pub fn all() -> Vec<(&'static str, fn())> {
    vec![("shell::signature_open_warning", signature_open_warning), ("shell::signature_relationship_selection", signature_relationship_selection), ("shell::signature_certificate_locations", signature_certificate_locations), ("shell::signature_certificate_policies", signature_certificate_policies)]
}

fn signature_open_warning() {
    use ibis_formats::ibis_ooxml::signatures::{inspect_signatures, ManifestDigest, ObjectDigest, SignatureValueCheck};
    let fixture = repo_root().join("crates/ibis-ooxml/tests/fixtures/signatures/signed-editable.docx");
    let dir = repo_root().join("target/scenario/signature-open-warning");
    std::fs::create_dir_all(&dir).unwrap();
    let signed = dir.join("signed.docx");
    std::fs::copy(fixture, &signed).unwrap();
    let package = Package::open(std::fs::read(&signed).unwrap()).unwrap();
    let before = inspect_signatures(&package).unwrap();
    assert_eq!(before[0].signature_value, SignatureValueCheck::MatchesUntrustedCertificate);
    assert_eq!(before[0].package_object, ObjectDigest::Matches);
    assert_eq!(before[0].manifest[0].digest, ManifestDigest::Matches);
    let mut scenario = Scenario::start_view();
    scenario.update(|e, w, cx| e.open_path(signed.clone(), None, w, cx));
    for _ in 0..200 {
        if scenario.with(|e, _| e.alert.is_some()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
        scenario.settle();
    }
    let alert = scenario.with(|e, _| e.alert.clone()).expect("signature warning after native open");
    assert!(alert.1.contains("有効性は確認していません"), "{}", alert.1);
    scenario.key("enter");
    scenario.menu("file.signature-inspect");
    for _ in 0..200 {
        if scenario.with(|e, _| e.alert.is_some()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
        scenario.settle();
    }
    let report = scenario.with(|e, _| e.alert.clone()).expect("signature inspection result");
    assert!(report.1.contains("証明書の信頼・失効"), "{}", report.1);
    assert!(report.1.contains("一致 1 件"), "{}", report.1);
    assert!(report.1.contains("その後の編集や保存は反映されません"), "{}", report.1);
    scenario.key("enter");
    let copy = dir.join("copy.docx");
    scenario.update(|e, w, _| e.save_to(copy.clone(), w)).unwrap();
    let copy_before = std::fs::read(&copy).unwrap();
    let copied = Package::open(copy_before.clone()).unwrap();
    assert_eq!(ibis_formats::ibis_ooxml::signatures::signature_parts(&copied).unwrap().len(), 1);
    assert_eq!(inspect_signatures(&copied).unwrap()[0].manifest[0].digest, ManifestDigest::Matches);
    scenario.key("cmd-down");
    scenario.type_text("edited");
    assert!(scenario.with(|e, _| e.is_dirty()), "document was edited");
    scenario.update(|e, w, cx| e.save(&crate::actions::Save, w, cx));
    assert!(scenario.with(|e, _| e.ask.is_some()), "edited signed document asks before saving");
    scenario.key("escape");
    assert!(scenario.with(|e, _| e.is_dirty()));
    assert_eq!(std::fs::read(&copy).unwrap(), copy_before, "cancel leaves the file unchanged");
    scenario.update(|e, w, cx| e.save(&crate::actions::Save, w, cx));
    assert!(scenario.with(|e, _| e.ask.is_some()));
    scenario.key("enter");
    assert!(!scenario.with(|e, _| e.is_dirty()), "confirm finishes saving");
    let after = Package::open(std::fs::read(&copy).unwrap()).unwrap();
    let report = inspect_signatures(&after).unwrap();
    assert_eq!(report[0].signature_value, SignatureValueCheck::MatchesUntrustedCertificate);
    assert_eq!(report[0].manifest[0].digest, ManifestDigest::Differs);
    scenario.update(|e, w, cx| e.save_copy(w, cx));
    assert!(scenario.with(|e, _| e.ask.is_some()), "copy asks even without unsaved changes");
    scenario.key("escape");
    scenario.type_text("again");
    assert!(scenario.with(|e, _| e.is_dirty()));
    let saved_before_cancel = std::fs::read(&copy).unwrap();
    scenario.update(|e, w, cx| e.save_copy(w, cx));
    assert!(scenario.with(|e, _| e.ask.as_ref().is_some_and(|a| a.message.contains("コピーや書き出し先"))), "copy asks about signatures");
    scenario.key("escape");
    scenario.update(|e, w, cx| e.export_pdf(w, cx));
    assert!(scenario.with(|e, _| e.ask.as_ref().is_some_and(|a| a.message.contains("コピーや書き出し先"))), "export asks about signatures");
    scenario.key("escape");
    scenario.update(|e, w, cx| e.set_protection(Some("test-password".to_string()), w, cx));
    assert!(scenario.with(|e, _| e.ask.is_some()), "password save asks about signatures");
    scenario.key("escape");
    assert!(scenario.with(|e, _| e.protection_password.is_none()), "cancelling leaves password unchanged");
    assert_eq!(std::fs::read(&copy).unwrap(), saved_before_cancel, "cancelled output actions do not change source");
    scenario.update(|e, w, cx| e.save_as(&crate::actions::SaveAs, w, cx));
    assert!(scenario.with(|e, _| e.ask.is_some()), "Save As also asks before showing a path dialog");
    scenario.key("escape");
    assert!(scenario.with(|e, _| e.is_dirty()));
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let marker = completed.clone();
    scenario.update(|e, w, cx| e.save_then(w, cx, move |_, _, _| marker.set(true)));
    assert!(scenario.with(|e, _| e.ask.is_some()));
    scenario.key("escape");
    assert!(!completed.get(), "cancel does not run the save continuation");
    assert!(scenario.with(|e, _| e.is_dirty()));
    let marker = completed.clone();
    scenario.update(|e, w, cx| e.save_then(w, cx, move |_, _, _| marker.set(true)));
    scenario.key("enter");
    assert!(completed.get(), "confirmed save runs the continuation");
    assert!(!scenario.with(|e, _| e.is_dirty()));
}

fn signature_relationship_selection() {
    let fixture = repo_root().join("crates/ibis-ooxml/tests/fixtures/signatures/relationship-selection-signed.docx");
    let mut scenario = Scenario::start_view();
    scenario.update(|e, w, cx| e.open_path(fixture.clone(), None, w, cx));
    for _ in 0..200 {
        if scenario.with(|e, _| e.alert.is_some()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
        scenario.settle();
    }
    let alert = scenario.with(|e, _| e.alert.clone()).expect("signature warning after native open");
    assert!(alert.1.contains("有効性は確認していません"), "{}", alert.1);
    scenario.key("enter");
    scenario.menu("file.signature-inspect");
    for _ in 0..200 {
        if scenario.with(|e, _| e.alert.is_some()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
        scenario.settle();
    }
    let report = scenario.with(|e, _| e.alert.clone()).expect("signature inspection result");
    assert!(report.1.contains("フィルター付き 1 件"), "{}", report.1);
    assert!(report.1.contains("選択 1 件・非選択 1 件"), "{}", report.1);
    assert!(report.1.contains("署名範囲の確定ではありません"), "{}", report.1);
}

fn signature_certificate_locations() {
    let fixture = repo_root().join("crates/ibis-ooxml/tests/fixtures/signatures/location-hints-signed.docx");
    let mut scenario = Scenario::start_view();
    scenario.update(|e, w, cx| e.open_path(fixture.clone(), None, w, cx));
    for _ in 0..200 {
        if scenario.with(|e, _| e.alert.is_some()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
        scenario.settle();
    }
    let alert = scenario.with(|e, _| e.alert.clone()).expect("signature warning after native open");
    assert!(alert.1.contains("有効性は確認していません"), "{}", alert.1);
    scenario.key("enter");
    scenario.menu("file.signature-inspect");
    for _ in 0..200 {
        if scenario.with(|e, _| e.alert.is_some()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
        scenario.settle();
    }
    let report = scenario.with(|e, _| e.alert.clone()).expect("signature inspection result");
    assert!(report.1.contains("証明書の参照先（未認証・未取得）"), "{}", report.1);
    assert!(report.1.contains("OCSP 1 件、CA 発行者 1 件、CRL 2 件"), "{}", report.1);
    assert!(report.1.contains("理由の制限がある CRL 配布点 1 件"), "{}", report.1);
    assert!(report.1.contains("失効状態と適用性は未確認"), "{}", report.1);
}

fn signature_certificate_policies() {
    use ibis_formats::ibis_ooxml::{base64, signatures::extract_single_embedded_certificate, SaveOptions};
    let fixtures = repo_root().join("crates/ibis-ooxml/tests/fixtures/signatures");
    let mut package = Package::open(std::fs::read(fixtures.join("signed-editable.docx")).unwrap()).unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let old = extract_single_embedded_certificate(&package, part).unwrap();
    let cert = std::fs::read(fixtures.join("certificate-policies.der")).unwrap();
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let changed = xml.replace(&base64::encode(&old), &base64::encode(&cert));
    assert_ne!(xml, changed);
    package.set_bytes(part, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-policy-scenario-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let mut scenario = Scenario::start_view();
    scenario.update(|e, w, cx| e.open_path(path.clone(), None, w, cx));
    for _ in 0..200 {
        if scenario.with(|e, _| e.alert.is_some()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
        scenario.settle();
    }
    assert!(scenario.with(|e, _| e.alert.is_some()));
    scenario.key("enter");
    scenario.menu("file.signature-inspect");
    for _ in 0..200 {
        if scenario.with(|e, _| e.alert.is_some()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
        scenario.settle();
    }
    let report = scenario.with(|e, _| e.alert.clone()).expect("signature inspection result");
    assert!(report.1.contains("証明書ポリシーの記載（未認証）"), "{}", report.1);
    assert!(report.1.contains("1.2.3.4.5"), "{}", report.1);
    assert!(report.1.contains("ポリシー適合性は未検証"), "{}", report.1);
    assert!(!report.1.contains("cps.example.invalid"));
    std::fs::remove_file(path).unwrap();
}
