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

#[test]
fn reconnects_a_bent_route_and_keeps_its_orthogonal_segments() {
    let doc = VsdxContainer::from_bytes(SAMPLE.into()).unwrap();
    let bytes = doc
        .edit_geometry(
            0,
            GeometryEdit::Reconnect {
                connector: 14,
                endpoint: Endpoint::End,
                target: 6,
            },
        )
        .unwrap();
    let reopened = VsdxContainer::from_bytes(bytes.clone().into()).unwrap();
    reopened.interpret_page_scene(0).unwrap();
    let zip = Archive::new(bytes.into()).unwrap();
    let entry = zip
        .entries()
        .iter()
        .find(|entry| entry.name == PAGE)
        .unwrap();
    let page = String::from_utf8(zip.read(entry).unwrap()).unwrap();
    let doc = roxmltree::Document::parse(&page).unwrap();
    let connector = doc
        .descendants()
        .find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some("14"))
        .unwrap();
    let vertices: Vec<[f64; 2]> = connector
        .descendants()
        .filter(|n| n.has_tag_name("Row") && matches!(n.attribute("T"), Some("MoveTo" | "LineTo")))
        .map(|row| {
            ["X", "Y"].map(|axis| {
                row.children()
                    .find(|c| c.attribute("N") == Some(axis))
                    .unwrap()
                    .attribute("V")
                    .unwrap()
                    .parse()
                    .unwrap()
            })
        })
        .collect();
    let [a, b, c, d] = vertices.as_slice() else {
        panic!("bend count changed");
    };
    assert_eq!(a[1], b[1]);
    assert_eq!(b[0], c[0]);
    assert_eq!(c[1], d[1]);
    assert!(a[0] < b[0] && b[0] < d[0] && a[1] < c[1]);
    assert!(
        (cell_value(&doc, "14", "BeginX") + d[0] - cell_value(&doc, "14", "EndX")).abs() < 1e-5
    );
    assert!(doc.descendants().any(|n| n.has_tag_name("Connect")
        && n.attribute("FromSheet") == Some("14")
        && n.attribute("FromCell") == Some("EndX")
        && n.attribute("ToSheet") == Some("6")));
}

#[test]
fn updates_vertical_and_leftward_routes() {
    let sample = include_bytes!("fixtures/07-zh-flowchart.vsdx");
    let doc = VsdxContainer::from_bytes(sample.as_slice().into()).unwrap();
    let moved = doc
        .edit_geometry(
            0,
            GeometryEdit::Move {
                shape: 5,
                dx: 0.0,
                dy: 0.1,
            },
        )
        .unwrap();
    let reopen = VsdxContainer::from_bytes(moved.into()).unwrap();
    let final_bytes = reopen
        .edit_geometry(
            0,
            GeometryEdit::Reconnect {
                connector: 19,
                endpoint: Endpoint::End,
                target: 13,
            },
        )
        .unwrap();
    VsdxContainer::from_bytes(final_bytes.clone().into())
        .unwrap()
        .interpret_page_scene(0)
        .unwrap();
    let zip = Archive::new(final_bytes.into()).unwrap();
    let page = zip
        .entries()
        .iter()
        .find(|entry| entry.name == PAGE)
        .unwrap();
    let page = String::from_utf8(zip.read(page).unwrap()).unwrap();
    let doc = roxmltree::Document::parse(&page).unwrap();
    assert!((cell_value(&doc, "14", "BeginY") - 11.409056).abs() < 1e-6);
    assert_eq!(
        cell_value(&doc, "14", "BeginX"),
        cell_value(&doc, "14", "EndX")
    );
    assert!(cell_value(&doc, "19", "EndX") < cell_value(&doc, "19", "BeginX"));
    assert!(doc.descendants().any(|n| n.has_tag_name("Connect")
        && n.attribute("FromSheet") == Some("19")
        && n.attribute("FromCell") == Some("EndX")
        && n.attribute("ToSheet") == Some("13")));
}

#[test]
fn reconnects_a_fixed_point_and_reopens_with_both_endpoint_formulas() {
    use ibis_ooxml::package::{Package, SaveOptions};
    let zip = Archive::new(SAMPLE.into()).unwrap();
    let entry = zip
        .entries()
        .iter()
        .find(|entry| entry.name == PAGE)
        .unwrap();
    let source = String::from_utf8(zip.read(entry).unwrap()).unwrap();
    let xml = roxmltree::Document::parse(&source).unwrap();
    let mut insertions = Vec::new();
    for (shape, x) in [("3", 0.0), ("4", 0.0)] {
        let node = xml
            .descendants()
            .find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some(shape))
            .unwrap();
        let y = cell_value(&xml, shape, "LocPinY");
        insertions.push((node.range().end - "</Shape>".len(),
            format!(r#"<Section N="Connection"><Row IX="0"><Cell N="X" V="{x}"/><Cell N="Y" V="{y}"/></Row></Section>"#)));
    }
    let mut source = source;
    insertions.sort_by_key(|&(offset, _)| std::cmp::Reverse(offset));
    for (offset, text) in insertions {
        source.insert_str(offset, &text);
    }
    let original = r#"<Connect FromSheet="12" FromCell="EndX" ToSheet="3"/>"#;
    assert!(source.contains(original));
    source = source.replace(original, r#"<Connect FromSheet="12" FromCell="EndX" ToSheet="3" ToCell="Connections.X1" ToPart="100"/>"#);
    source = source.replacen(
        "_WALKGLUE(EndTrigger,BegTrigger,WalkPreference)",
        "PAR(PNT(Sheet.3!Connections.X1,Sheet.3!Connections.Y1))",
        2,
    );
    let mut package = Package::open(SAMPLE).unwrap();
    package.set_bytes(PAGE, source.into_bytes());
    let doc =
        VsdxContainer::from_bytes(package.save(SaveOptions::default()).unwrap().into()).unwrap();
    let bytes = doc
        .edit_geometry(
            0,
            GeometryEdit::Reconnect {
                connector: 12,
                endpoint: Endpoint::End,
                target: 4,
            },
        )
        .unwrap();
    VsdxContainer::from_bytes(bytes.clone().into())
        .unwrap()
        .interpret_page_scene(0)
        .unwrap();
    let zip = Archive::new(bytes.into()).unwrap();
    let entry = zip
        .entries()
        .iter()
        .find(|entry| entry.name == PAGE)
        .unwrap();
    let result = String::from_utf8(zip.read(entry).unwrap()).unwrap();
    assert_eq!(
        result
            .matches("PAR(PNT(Sheet.4!Connections.X1,Sheet.4!Connections.Y1))")
            .count(),
        2
    );
    assert!(result.contains(r#"<Connect FromSheet="12" FromCell="EndX" ToSheet="4" ToCell="Connections.X1" ToPart="100"/>"#));
}

fn rewrite_flowchart(change: impl FnOnce(String) -> String) -> VsdxContainer {
    use ibis_ooxml::package::{Package, SaveOptions};
    let zip = Archive::new(SAMPLE.into()).unwrap();
    let entry = zip
        .entries()
        .iter()
        .find(|entry| entry.name == PAGE)
        .unwrap();
    let source = String::from_utf8(zip.read(entry).unwrap()).unwrap();
    let mut package = Package::open(SAMPLE).unwrap();
    package.set_bytes(PAGE, change(source).into_bytes());
    VsdxContainer::from_bytes(package.save(SaveOptions::default()).unwrap().into()).unwrap()
}

#[test]
fn reconnects_relative_cubic_geometry_without_discarding_control_points() {
    let doc = rewrite_flowchart(|mut source| {
        let xml = roxmltree::Document::parse(&source).unwrap();
        let route = xml
            .descendants()
            .find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some("14"))
            .unwrap();
        let geometry = route
            .children()
            .find(|n| n.attribute("N") == Some("Geometry"))
            .unwrap()
            .range();
        source.replace_range(geometry, concat!(
            r#"<Section N="Geometry" IX="0"><Cell N="NoFill" V="1"/>"#,
            r#"<Row T="RelMoveTo" IX="1"><Cell N="X" V="0"/><Cell N="Y" V="0"/></Row>"#,
            r#"<Row T="RelCubBezTo" IX="2"><Cell N="X" V="1"/><Cell N="Y" V="1"/>"#,
            r#"<Cell N="A" V="0.25"/><Cell N="B" V="0"/><Cell N="C" V="0.75"/><Cell N="D" V="1"/></Row></Section>"#));
        source
    });
    let bytes = doc
        .edit_geometry(
            0,
            GeometryEdit::Reconnect {
                connector: 14,
                endpoint: Endpoint::End,
                target: 6,
            },
        )
        .unwrap();
    let reopened = VsdxContainer::from_bytes(bytes.clone().into()).unwrap();
    reopened.interpret_page_scene(0).unwrap();
    let zip = Archive::new(bytes.into()).unwrap();
    let page = zip
        .entries()
        .iter()
        .find(|entry| entry.name == PAGE)
        .unwrap();
    let page = String::from_utf8(zip.read(page).unwrap()).unwrap();
    assert!(page.contains(r#"<Row T="RelCubBezTo" IX="2"><Cell N="X" V="1"/><Cell N="Y" V="1"/><Cell N="A" V="0.25"/><Cell N="B" V="0"/><Cell N="C" V="0.75"/><Cell N="D" V="1"/>"#));
    assert!(page.contains(r#"FromSheet="14" FromCell="EndX" ToSheet="6""#));
}

#[test]
fn moves_an_unconnected_child_in_group_coordinates() {
    let doc = rewrite_flowchart(|mut source| {
        let xml = roxmltree::Document::parse(&source).unwrap();
        let shape = xml
            .descendants()
            .find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some("1"))
            .unwrap()
            .range();
        source.insert_str(shape.end, "</Shapes></Shape>");
        source.insert_str(shape.start, r#"<Shape ID="100" Type="Group"><Cell N="PinX" V="0"/><Cell N="PinY" V="0"/><Cell N="LocPinX" V="0"/><Cell N="LocPinY" V="0"/><Cell N="Width" V="20"/><Cell N="Height" V="3"/><Shapes>"#);
        source
    });
    let bytes = doc
        .edit_geometry(
            0,
            GeometryEdit::Move {
                shape: 1,
                dx: 0.25,
                dy: 0.0,
            },
        )
        .unwrap();
    let reopened = VsdxContainer::from_bytes(bytes.clone().into()).unwrap();
    reopened.interpret_page_scene(0).unwrap();
    let zip = Archive::new(bytes.into()).unwrap();
    let page = zip
        .entries()
        .iter()
        .find(|entry| entry.name == PAGE)
        .unwrap();
    let page = String::from_utf8(zip.read(page).unwrap()).unwrap();
    let xml = roxmltree::Document::parse(&page).unwrap();
    assert!((cell_value(&xml, "1", "PinX") - 10.181102).abs() < 1e-6);
    assert_eq!(cell_value(&xml, "100", "PinX"), 0.0);
}
