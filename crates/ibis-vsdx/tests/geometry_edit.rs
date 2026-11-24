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
    verify_fixed_point(false);
}

#[test]
fn reconnects_to_a_master_connection_point_using_instance_dimensions() {
    verify_fixed_point(true);
}

fn verify_fixed_point(inherited: bool) {
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
    for shape in ["3", "4"] {
        let node = xml
            .descendants()
            .find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some(shape))
            .unwrap();
        let y = cell_value(&xml, shape, "LocPinY");
        if inherited && shape == "4" {
            insertions.push((
                node.range().start + "<Shape".len(),
                r#" Master="100""#.to_owned(),
            ));
        } else {
            insertions.push((node.range().end - "</Shape>".len(),
                format!(r#"<Section N="Connection"><Row IX="0"><Cell N="X" V="0"/><Cell N="Y" V="{y}"/></Row></Section>"#)));
        }
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
    if inherited {
        for (name, closing, addition) in [
            (
                "visio/masters/masters.xml",
                "</Masters>",
                r#"<Master ID="100"><Rel r:id="point-master"/></Master>"#,
            ),
            (
                "visio/masters/_rels/masters.xml.rels",
                "</Relationships>",
                r#"<Relationship Id="point-master" Type="http://schemas.microsoft.com/visio/2010/relationships/master" Target="point-master.xml"/>"#,
            ),
        ] {
            let entry = zip
                .entries()
                .iter()
                .find(|entry| entry.name == name)
                .unwrap();
            let original = String::from_utf8(zip.read(entry).unwrap()).unwrap();
            assert!(original.contains(closing));
            package.set_bytes(
                name,
                original
                    .replace(closing, &format!("{addition}{closing}"))
                    .into_bytes(),
            );
        }
        package.set_bytes("visio/masters/point-master.xml", br#"<MasterContents xmlns="http://schemas.microsoft.com/office/visio/2012/main"><Shapes><Shape ID="1"><Cell N="Height" V="2"/><Section N="Connection"><Row IX="0"><Cell N="X" V="0"/><Cell N="Y" V="1" F="0.5*Height"/></Row></Section></Shape></Shapes></MasterContents>"#.to_vec());
    }
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
    let page = roxmltree::Document::parse(&result).unwrap();
    let expected_x = cell_value(&page, "4", "PinX") - cell_value(&page, "4", "LocPinX");
    assert!((cell_value(&page, "12", "EndX") - expected_x).abs() < 1e-6);
    assert!((cell_value(&page, "12", "EndY") - cell_value(&page, "4", "PinY")).abs() < 1e-5);
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
        source.insert_str(shape.start, r#"<Shape ID="100" Type="Group"><Cell N="PinX" V="1"/><Cell N="PinY" V="0.5"/><Cell N="LocPinX" V="0"/><Cell N="LocPinY" V="0"/><Cell N="Width" V="20"/><Cell N="Height" V="3"/><Shapes>"#);
        source
    });
    let targets = doc.geometry_targets(0).unwrap();
    let child = targets.group_children.iter().find(|child| child.id == 1).unwrap().bounds;
    assert!(!targets.shapes.iter().any(|(id, _)| *id == 1));
    assert!(((child.x0 + child.x1) / 2.0 - (9.931102 + 1.0) * 72.0).abs() < 0.01);
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
    assert_eq!(cell_value(&xml, "100", "PinX"), 1.0);
}

#[test]
fn group_drag_inverts_the_composed_rotation_and_reflection() {
    let doc = rewrite_flowchart(|mut source| {
        let xml = roxmltree::Document::parse(&source).unwrap();
        let shape = xml.descendants()
            .find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some("1"))
            .unwrap().range();
        source.insert_str(shape.end, "</Shapes></Shape></Shapes></Shape>");
        source.insert_str(shape.start, concat!(
            r#"<Shape ID="100" Type="Group"><Cell N="PinX" V="12"/><Cell N="PinY" V="1"/><Cell N="LocPinX" V="0"/><Cell N="LocPinY" V="0"/><Cell N="Width" V="20"/><Cell N="Height" V="3"/><Cell N="Angle" V="1.5707963267948966"/><Shapes>"#,
            r#"<Shape ID="101" Type="Group"><Cell N="PinX" V="1"/><Cell N="PinY" V="2"/><Cell N="LocPinX" V="0"/><Cell N="LocPinY" V="0"/><Cell N="Width" V="20"/><Cell N="Height" V="3"/><Cell N="FlipX" V="1"/><Shapes>"#,
        ));
        source
    });
    let before = doc.geometry_targets(0).unwrap();
    let child = before.group_children.iter().find(|child| child.id == 1).unwrap();
    let cx = (child.bounds.x0 + child.bounds.x1) / 2.0;
    let cy = (child.bounds.y0 + child.bounds.y1) / 2.0;
    assert!(child.contains(cx, cy));
    let (dx, dy) = child.local_delta(0.25, 0.125);
    assert!((dx + 0.125).abs() < 1e-9 && (dy + 0.25).abs() < 1e-9);
    let bytes = doc.edit_geometry(0, GeometryEdit::Move { shape: 1, dx, dy }).unwrap();
    let reopened = VsdxContainer::from_bytes(bytes.into()).unwrap();
    let after = reopened.geometry_targets(0).unwrap();
    let moved = after.group_children.iter().find(|child| child.id == 1).unwrap();
    let parent = |targets: &ibis_vsdx::geometry_edit::GeometryTargets| {
        targets.group_children.iter().find(|child| child.id == 101).unwrap().bounds
    };
    assert_eq!(parent(&before), parent(&after));
    assert!((moved.bounds.x0 - child.bounds.x0 - 18.0).abs() < 0.01);
    assert!((moved.bounds.y0 - child.bounds.y0 + 9.0).abs() < 0.01);
}

#[test]
fn reconnects_circular_and_elliptical_arcs_at_uniform_scale() {
    use ibis_pdf::{DisplayCmd, scene::PathCmd};

    for arc_type in ["ArcTo", "EllipticalArcTo", "RelEllipticalArcTo"] {
        let doc = rewrite_flowchart(|mut source| {
            let xml = roxmltree::Document::parse(&source).unwrap();
            let route = xml
                .descendants()
                .find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some("12"))
                .unwrap();
            let geometry = route
                .children()
                .find(|n| n.attribute("N") == Some("Geometry"))
                .unwrap()
                .range();
            let (move_type, middle, end, controls) = match arc_type {
                "ArcTo" => ("MoveTo", 0.005, 0.629921, r#"<Cell N="A" V="-0.195"/>"#),
                "RelEllipticalArcTo" => (
                    "RelMoveTo",
                    0.5,
                    1.0,
                    r#"<Cell N="A" V="0.5"/><Cell N="B" V="20"/><Cell N="C" V="0.2"/><Cell N="D" V="2"/>"#,
                ),
                _ => (
                    "MoveTo",
                    0.005,
                    0.629921,
                    r#"<Cell N="A" V="0.3149605"/><Cell N="B" V="0.2"/><Cell N="C" V="0.2"/><Cell N="D" V="2"/>"#,
                ),
            };
            source.replace_range(geometry, &format!(concat!(
                r#"<Section N="Geometry" IX="0"><Cell N="NoFill" V="1"/>"#,
                r#"<Row T="{}" IX="1"><Cell N="X" V="0"/><Cell N="Y" V="{}"/></Row>"#,
                r#"<Row T="{}" IX="2"><Cell N="X" V="{}"/><Cell N="Y" V="{}"/>{}</Row></Section>"#),
                move_type, middle, arc_type, end, middle, controls));
            source
        });
        let curve = |page: &ibis_pdf::PageScene| {
            page.display_list
                .iter()
                .find_map(|cmd| {
                    let DisplayCmd::DrawPath { commands, .. } = cmd else {
                        return None;
                    };
                    let Some(PathCmd::MoveTo(x, _)) = commands.first() else {
                        return None;
                    };
                    if (*x - 1.909449 * 72.0).abs() > 0.01 {
                        return None;
                    }
                    let values: Vec<_> = commands
                        .iter()
                        .filter_map(|path| match *path {
                            PathCmd::CubicTo(a, b, c, d, x, y) => Some([a, b, c, d, x, y]),
                            _ => None,
                        })
                        .collect();
                    (!values.is_empty()).then_some(values)
                })
                .unwrap()
        };
        let before = curve(&doc.interpret_page_scene(0).unwrap());
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
        let path =
            std::env::temp_dir().join(format!("ibis-arc-{arc_type}-{}.vsdx", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let opened = VsdxContainer::from_bytes(bytes.clone().into()).unwrap();
        let scene = opened.interpret_page_scene(0).unwrap();
        let after = curve(&scene);
        let origin = [
            1.909449 * 72.0,
            scene.info.crop_box.height() - 0.639764 * 72.0,
        ];
        let scale =
            (after.last().unwrap()[4] - origin[0]) / (before.last().unwrap()[4] - origin[0]);
        assert!(scale > 2.0);
        assert_eq!(before.len(), after.len());
        for (a, b) in before.into_iter().zip(after) {
            for i in 0..6 {
                assert!(
                    (b[i] - (origin[i % 2] + (a[i] - origin[i % 2]) * scale)).abs() < 0.015,
                    "{a:?} {b:?}"
                );
            }
        }
        let zip = Archive::new(bytes.into()).unwrap();
        let page = zip
            .entries()
            .iter()
            .find(|entry| entry.name == PAGE)
            .unwrap();
        let xml = String::from_utf8(zip.read(page).unwrap()).unwrap();
        assert!(xml.contains(r#"FromSheet="12" FromCell="EndX" ToSheet="4""#));
    }
}
