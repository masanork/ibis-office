// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_formats::zip::Archive;
use ibis_pdf::{DisplayCmd, PdfRect, scene::PathCmd};
use ibis_vsdx::{
    VsdxContainer, is_vsdx,
    write::{Drawing, ShapeKind, Side},
};

fn frame(x0: f32, y0: f32, x1: f32, y1: f32) -> PdfRect {
    PdfRect { x0, y0, x1, y1 }
}

#[test]
fn creates_a_blank_page_that_can_be_reopened() {
    let bytes = Drawing::new(420.0, 240.0).unwrap().to_vsdx().unwrap();
    assert!(is_vsdx(&bytes));
    let doc = VsdxContainer::from_bytes(bytes.into()).unwrap();
    let pages = doc.page_infos();
    assert_eq!(pages.len(), 1);
    assert_eq!(
        (pages[0].crop_box.width(), pages[0].crop_box.height()),
        (420.0, 240.0)
    );
    assert!(doc.interpret_page_scene(0).unwrap().display_list.is_empty());
}

#[test]
fn shape_move_updates_connector_position_and_keeps_connection_points() {
    let mut drawing = Drawing::new(420.0, 240.0).unwrap();
    let a = drawing
        .add_shape(
            ShapeKind::Rectangle,
            frame(20.0, 60.0, 130.0, 120.0),
            "受付 & 確認",
            [220, 236, 252],
        )
        .unwrap();
    let b = drawing
        .add_shape(
            ShapeKind::Ellipse,
            frame(210.0, 60.0, 330.0, 120.0),
            "処理完了",
            [220, 250, 235],
        )
        .unwrap();
    drawing.connect((a, Side::Right), (b, Side::Left)).unwrap();
    drawing
        .move_shape(b, frame(260.0, 60.0, 380.0, 120.0))
        .unwrap();
    let bytes = drawing.to_vsdx().unwrap();
    let zip = Archive::new(&bytes).unwrap();
    let xml = String::from_utf8(
        zip.read_bounded("visio/pages/page1.xml", 10_000)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let xml = roxmltree::Document::parse(&xml).unwrap();
    let connect: Vec<_> = xml
        .descendants()
        .filter(|n| n.tag_name().name() == "Connect")
        .collect();
    assert_eq!(connect.len(), 2);
    assert_eq!(connect[0].attribute("ToSheet"), Some("1"));
    assert_eq!(connect[0].attribute("ToCell"), Some("Connections.X2"));
    assert_eq!(connect[1].attribute("ToSheet"), Some("2"));
    assert_eq!(connect[1].attribute("ToCell"), Some("Connections.X1"));
    assert!(xml.descendants().any(|n| n.attribute("N") == Some("EndX")
        && n.attribute("F") == Some("PAR(PNT(Sheet.2!Connections.X1,Sheet.2!Connections.Y1))")));

    let doc = VsdxContainer::from_bytes(bytes.into()).unwrap();
    let scene = doc.interpret_page_scene(0).unwrap();
    let DisplayCmd::DrawPath { commands, .. } = &scene.display_list[0] else {
        panic!("expected connector under the two nodes");
    };
    assert!(matches!(
        commands.as_slice(),
        [PathCmd::MoveTo(130.0, 90.0), PathCmd::LineTo(260.0, 90.0)]
    ));
    assert!(scene.text_index.plain_text().contains("受付 & 確認"));
    assert!(scene.text_index.plain_text().contains("処理完了"));
}

#[test]
fn rejects_impossible_shapes_and_connections_before_writing() {
    assert!(Drawing::new(f64::NAN, 200.0).is_err());
    let mut drawing = Drawing::new(420.0, 240.0).unwrap();
    assert!(
        drawing
            .add_shape(ShapeKind::Diamond, frame(0.0, 0.0, 600.0, 50.0), "", [0; 3])
            .is_err()
    );
    assert!(
        drawing
            .add_shape(
                ShapeKind::Diamond,
                frame(0.0, 0.0, 50.0, 50.0),
                "\0",
                [0; 3]
            )
            .is_err()
    );
    let a = drawing
        .add_shape(
            ShapeKind::Diamond,
            frame(20.0, 20.0, 80.0, 80.0),
            "",
            [0; 3],
        )
        .unwrap();
    assert!(drawing.connect((a, Side::Left), (a, Side::Right)).is_err());
}
