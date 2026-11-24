// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_ooxml::zip::{Archive, EntryMeta, Writer};
use ibis_vsdx::VsdxContainer;

const SAMPLE: &[u8] = include_bytes!("fixtures/08-multi-page-handbook.vsdx");

fn package_with_extra_parts(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let source = Archive::new(SAMPLE.into()).unwrap();
    let mut zip = Writer::new();
    zip.set_comment(b"original comment");
    for entry in source.entries() {
        zip.copy(&source, entry).unwrap();
    }
    for &(name, content) in extra {
        zip.add(name, EntryMeta::new(), content).unwrap();
    }
    zip.finish().unwrap()
}

#[test]
fn replaces_one_label_without_changing_other_package_parts() {
    let input = package_with_extra_parts(&[("custom/opaque.dat", b"arbitrary vendor payload")]);
    let doc = VsdxContainer::from_bytes(input.clone().into()).unwrap();
    let labels = doc.editable_labels(1).unwrap();
    let title = labels
        .iter()
        .find(|label| label.text == "2. Auth & Token Flow")
        .unwrap();
    assert_eq!(
        doc.replace_label(1, title.shape_id, &title.text).unwrap(),
        input
    );
    let replacement = "2. 認証 & <トークン>";
    let edited = doc.replace_label(1, title.shape_id, replacement).unwrap();
    let before = Archive::new(input.clone().into()).unwrap();
    let after = Archive::new(edited.clone().into()).unwrap();
    assert_eq!(before.comment(), after.comment());
    assert_eq!(before.entries().len(), after.entries().len());
    for (old, new) in before.entries().iter().zip(after.entries()) {
        assert_eq!(old.name, new.name);
        if old.name == "visio/pages/page2.xml" {
            let old = String::from_utf8(before.read(old).unwrap()).unwrap();
            let new = String::from_utf8(after.read(new).unwrap()).unwrap();
            assert_eq!(
                new,
                old.replacen(
                    "2. Auth &amp; Token Flow",
                    "2. 認証 &amp; &lt;トークン&gt;",
                    1
                )
            );
        } else {
            assert_eq!(
                before.raw(old),
                after.raw(new),
                "untouched part {}",
                old.name
            );
        }
    }
    let opened = VsdxContainer::from_bytes(edited.into()).unwrap();
    assert!(
        opened
            .interpret_page_scene(1)
            .unwrap()
            .text_index
            .plain_text()
            .contains(replacement)
    );
    assert_eq!(
        opened
            .editable_labels(1)
            .unwrap()
            .into_iter()
            .find(|label| label.shape_id == title.shape_id)
            .unwrap()
            .text,
        replacement
    );
}

#[test]
fn refuses_unknown_shapes_invalid_text_and_signed_documents() {
    let doc = VsdxContainer::from_bytes(SAMPLE.into()).unwrap();
    let id = doc.editable_labels(0).unwrap()[0].shape_id;
    assert!(doc.replace_label(0, u32::MAX, "another label").is_err());
    assert!(doc.replace_label(0, id, "\0").is_err());
    let signed = package_with_extra_parts(&[("_xmlsignatures/sig1.xml", b"<Signature/>")]);
    let doc = VsdxContainer::from_bytes(signed.into()).unwrap();
    assert!(doc.replace_label(0, id, "another label").is_err());
}

#[test]
fn patches_multiple_labels_by_shape_id_in_one_save() {
    let doc = VsdxContainer::from_bytes(SAMPLE.into()).unwrap();
    let labels = doc.editable_labels(1).unwrap();
    let first = labels[0].shape_id;
    let last = labels.last().unwrap().shape_id;
    let updated = doc
        .replace_labels(1, &[(last, "After"), (first, "Before & verified")])
        .unwrap();
    let opened = VsdxContainer::from_bytes(updated.into()).unwrap();
    let labels = opened.editable_labels(1).unwrap();
    assert_eq!(labels[0].text, "Before & verified");
    assert_eq!(labels.last().unwrap().text, "After");
    assert!(
        doc.replace_labels(1, &[(first, "one"), (first, "two")])
            .is_err()
    );
    assert!(
        doc.replace_labels(1, &[(first, "valid"), (u32::MAX, "unknown")])
            .is_err()
    );
}

#[test]
fn keeps_mixed_format_labels_out_of_the_edit_list() {
    let source = Archive::new(SAMPLE.into()).unwrap();
    let mut zip = Writer::new();
    for entry in source.entries() {
        if entry.name == "visio/pages/page2.xml" {
            let xml = String::from_utf8(source.read(entry).unwrap()).unwrap();
            let mixed = xml.replacen(
                "2. Auth &amp; Token Flow",
                r#"2. Auth <cp IX="1"/>&amp; Token Flow"#,
                1,
            );
            assert_ne!(mixed, xml);
            zip.add(&entry.name, entry.meta, mixed.as_bytes()).unwrap();
        } else {
            zip.copy(&source, entry).unwrap();
        }
    }
    let before = VsdxContainer::from_bytes(SAMPLE.into()).unwrap();
    let shape = before
        .editable_labels(1)
        .unwrap()
        .into_iter()
        .find(|label| label.text == "2. Auth & Token Flow")
        .unwrap()
        .shape_id;
    let mixed = VsdxContainer::from_bytes(zip.finish().unwrap().into()).unwrap();
    assert!(
        !mixed
            .editable_labels(1)
            .unwrap()
            .iter()
            .any(|label| label.shape_id == shape)
    );
    assert!(mixed.replace_label(1, shape, "replacement").is_err());
}

#[test]
fn saves_across_pages_and_can_restore_a_cleared_label() {
    let original = VsdxContainer::from_bytes(SAMPLE.into()).unwrap();
    let first = original.editable_labels(0).unwrap().remove(0);
    let second = original.editable_labels(1).unwrap().remove(0);
    let updated = original
        .replace_document_labels([(0, first.shape_id, ""), (1, second.shape_id, "ページ2")])
        .unwrap();
    assert!(
        original
            .replace_document_labels([(0, first.shape_id, "ok"), (1, u32::MAX, "bad")])
            .is_err()
    );
    let reopened = VsdxContainer::from_bytes(updated.into()).unwrap();
    assert_eq!(reopened.editable_labels(0).unwrap()[0].text, "");
    assert_eq!(reopened.editable_labels(1).unwrap()[0].text, "ページ2");
    assert!(
        !reopened
            .interpret_page_scene(0)
            .unwrap()
            .text_index
            .plain_text()
            .contains(&first.text)
    );
    let restored = reopened
        .replace_label(0, first.shape_id, &first.text)
        .unwrap();
    let reopened = VsdxContainer::from_bytes(restored.into()).unwrap();
    assert_eq!(reopened.editable_labels(0).unwrap()[0], first);
    assert_eq!(reopened.editable_labels(1).unwrap()[0].text, "ページ2");
}
