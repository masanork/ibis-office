// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_ooxml::zip::Archive;
use ibis_vsdx::{
    VsdxContainer,
    geometry_edit::{Endpoint, GeometryEdit},
};

const SAMPLE: &[u8] = include_bytes!("fixtures/01-basic-flowchart.vsdx");
const PAGE: &str = "visio/pages/page1.xml";

fn cell_value(doc: &roxmltree::Document<'_>, shape: &str, name: &str) -> f64 {
    doc.descendants()
        .find(|node| node.has_tag_name("Shape") && node.attribute("ID") == Some(shape))
        .unwrap()
        .children()
        .find(|node| node.has_tag_name("Cell") && node.attribute("N") == Some(name))
        .unwrap()
        .attribute("V")
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn reopens_move_and_reconnect_edits_and_preserves_other_parts() {
    let original = VsdxContainer::from_bytes(SAMPLE.into()).unwrap();
    let moved = original
        .edit_geometry(
            0,
            GeometryEdit::Move {
                shape: 2,
                dx: 0.25,
                dy: 0.0,
            },
        )
        .unwrap();
    let reopened = VsdxContainer::from_bytes(moved.into()).unwrap();
    let edited = reopened
        .edit_geometry(
            0,
            GeometryEdit::Reconnect {
                connector: 12,
                endpoint: Endpoint::End,
                target: 4,
            },
        )
        .unwrap();
    let reopened = VsdxContainer::from_bytes(edited.clone().into()).unwrap();
    assert_eq!(reopened.page_infos().len(), 1);
    let scene = reopened.interpret_page_scene(0).unwrap();
    assert!(scene.text_index.plain_text().contains("Refund requested"));

    let before = Archive::new(SAMPLE.into()).unwrap();
    let after = Archive::new(edited.into()).unwrap();
    for entry in before.entries() {
        let copy = after
            .entries()
            .iter()
            .find(|e| e.name == entry.name)
            .unwrap();
        if entry.name == PAGE {
            let xml = String::from_utf8(after.read(copy).unwrap()).unwrap();
            let xml = roxmltree::Document::parse(&xml).unwrap();
            assert!((cell_value(&xml, "2", "PinX") - 1.322835).abs() < 1e-6);
            assert!((cell_value(&xml, "12", "BeginX") - 2.159449).abs() < 1e-6);
            assert!(cell_value(&xml, "12", "EndX") > 4.0);
            assert!(xml.descendants().any(|node| node.has_tag_name("Connect")
                && node.attribute("FromSheet") == Some("12")
                && node.attribute("FromCell") == Some("EndX")
                && node.attribute("ToSheet") == Some("4")));
            assert!(xml.descendants().any(|node| node.attribute("F")
                == Some("_XFTRIGGER(Sheet.4!EventXFMod)")
                && node.attribute("N") == Some("EndTrigger")));
        } else {
            assert_eq!(
                before.read(entry).unwrap(),
                after.read(copy).unwrap(),
                "{}",
                entry.name
            );
        }
    }
}

#[test]
fn leaves_complex_routes_and_invalid_targets_untouched() {
    let doc = VsdxContainer::from_bytes(SAMPLE.into()).unwrap();
    assert!(
        doc.edit_geometry(
            0,
            GeometryEdit::Move {
                shape: 2,
                dx: 0.0,
                dy: 1.0
            }
        )
        .is_err()
    );
    assert!(
        doc.edit_geometry(
            0,
            GeometryEdit::Reconnect {
                connector: 14,
                endpoint: Endpoint::End,
                target: 2
            }
        )
        .is_err()
    );
    assert!(
        doc.edit_geometry(
            0,
            GeometryEdit::Reconnect {
                connector: 12,
                endpoint: Endpoint::End,
                target: 2
            }
        )
        .is_err()
    );
}
