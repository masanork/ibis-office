// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_pdf::{DisplayCmd, PageScene, scene::PathCmd};
use ibis_vsdx::VsdxContainer;

fn open(bytes: &'static [u8]) -> VsdxContainer {
    assert!(ibis_vsdx::is_vsdx(bytes));
    VsdxContainer::from_bytes(bytes.into()).unwrap()
}

fn assert_label(scene: &PageScene, label: &str) {
    assert!(
        scene.display_list.iter().any(|cmd| {
            matches!(cmd, DisplayCmd::DrawStyledGlyphRun { text, .. } if text == label)
        }),
        "missing label {label:?} on page {}",
        scene.info.index + 1
    );
    let searchable: String = scene.text_index.chars.iter().map(|c| c.ch).collect();
    assert!(searchable.contains(label));
}

fn near(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 0.05
}

#[test]
fn refund_flowchart_renders_shapes_connector_and_labels() {
    let document = open(include_bytes!("fixtures/01-basic-flowchart.vsdx"));
    assert_eq!(document.page_infos().len(), 1);
    let scene = document.interpret_page_scene(0).unwrap();
    assert_label(&scene, "Customer Refund Process");
    assert_label(&scene, "Within 30");
    assert_label(&scene, "days?");
    assert_label(&scene, "yes");
    assert!(
        scene.display_list.iter().any(|cmd| {
            matches!(cmd, DisplayCmd::DrawPath { commands, .. }
            if commands.iter().any(|cmd| matches!(cmd, PathCmd::CubicTo(..))))
        }),
        "missing curved outline"
    );
    let page_height = scene.info.crop_box.height();
    let connector_y = page_height - 0.639764 * 72.0;
    assert!(
        scene.display_list.iter().any(|cmd| {
            matches!(cmd, DisplayCmd::DrawPath { fill_rgba: None, commands, .. }
            if matches!(commands.as_slice(), [PathCmd::MoveTo(x0, y0), .., PathCmd::LineTo(x1, y1)]
                if near(*x0, 1.909449 * 72.0) && near(*x1, 2.53937 * 72.0)
                    && near(*y0, connector_y) && near(*y1, connector_y)))
        }),
        "missing connection between refund request and order lookup"
    );
}

#[test]
fn handbook_keeps_page_order_and_mixed_page_sizes() {
    let document = open(include_bytes!("fixtures/08-multi-page-handbook.vsdx"));
    let pages = document.page_infos();
    assert_eq!(pages.len(), 3);
    assert!(pages[0].crop_box.width() > pages[0].crop_box.height());
    assert!(pages[2].crop_box.width() < pages[2].crop_box.height());
    for (index, title) in [
        "1. System Overview",
        "2. Auth & Token Flow",
        "3. Incident Triage",
    ]
    .into_iter()
    .enumerate()
    {
        let scene = document.interpret_page_scene(index).unwrap();
        assert_eq!(scene.info.index, index);
        assert_label(&scene, title);
        if index == 2 {
            assert!(scene.display_list.windows(2).any(|pair| matches!(pair,
                [DisplayCmd::DrawPath { fill_rgba: Some([255, 255, 255, 255]), stroke_rgba: None, .. },
                 DisplayCmd::DrawStyledGlyphRun { text, .. }] if text == "yes (SEV-1/2)"
            )));
        }
        assert!(
            scene
                .display_list
                .iter()
                .any(|cmd| matches!(cmd, DisplayCmd::DrawPath { .. }))
        );
    }
}

#[test]
fn sample_details_keep_their_styles_and_geometry() {
    use ibis_pdf::scene::TextAnchor;

    let document = open(include_bytes!("fixtures/01-basic-flowchart.vsdx"));
    let scene = document.interpret_page_scene(0).unwrap();
    let display = &scene.display_list;
    assert!(display.iter().any(|cmd| matches!(cmd,
        DisplayCmd::DrawStyledGlyphRun {
            text, anchor: TextAnchor::Start, origin: (x, _), weight: 700,
            rgba: [15, 23, 42, 255], ..
        } if text == "Customer Refund Process" && near(*x, 18.43)
    )));
    assert!(display.iter().any(|cmd| matches!(cmd,
        DisplayCmd::DrawStyledGlyphRun { text, anchor: TextAnchor::Center, .. }
        if text == "Look up order"
    )));
    assert!(scene.text_index.plain_text().contains("Within 30 days?"));

    let count_curves = |color| {
        display
            .iter()
            .filter_map(|cmd| match cmd {
                DisplayCmd::DrawPath {
                    stroke_rgba: Some(rgba),
                    commands,
                    ..
                } if *rgba == color => Some(
                    commands
                        .iter()
                        .filter(|cmd| matches!(cmd, PathCmd::CubicTo(..)))
                        .count(),
                ),
                _ => None,
            })
            .sum::<usize>()
    };
    // The cylindrical ledger has three arcs and the order box has four rounded corners.
    assert!(count_curves([100, 116, 139, 255]) >= 6);
    assert!(count_curves([108, 142, 191, 255]) >= 4);
    assert!(display.iter().any(|cmd| matches!(cmd,
        DisplayCmd::DrawPath { fill_rgba: None, stroke_rgba: Some([71, 85, 105, 255]), commands, .. }
        if commands.iter().filter(|cmd| matches!(cmd, PathCmd::MoveTo(..))).count() > 10
    )));
    let arrows = display.iter().filter(|cmd| matches!(cmd,
        DisplayCmd::DrawPath { fill_rgba: Some([71, 85, 105, 255]), stroke_rgba: None, commands, .. }
        if matches!(commands.as_slice(), [PathCmd::MoveTo(..), PathCmd::LineTo(..), PathCmd::LineTo(..), PathCmd::Close])
    )).count();
    assert_eq!(arrows, 12);
}

#[test]
fn chinese_flowchart_wraps_at_a_legal_boundary_and_keeps_search_positions() {
    let document = open(include_bytes!("fixtures/07-zh-flowchart.vsdx"));
    assert_eq!(document.page_infos().len(), 1);
    let scene = document.interpret_page_scene(0).unwrap();
    for label in [
        "IT 运维支持",
        "核验入职材料",
        "并建档",
        "转正答辩通",
        "过？",
    ] {
        assert_label(&scene, label);
    }
    let phrase = "转正答辩通过？";
    let byte = scene.text_index.plain_text().find(phrase).unwrap();
    let start = scene.text_index.plain_text()[..byte].chars().count();
    let boxes = &scene.text_index.chars[start..start + phrase.chars().count()];
    let (first, second) = boxes.split_at(5);
    assert!(
        first
            .iter()
            .all(|c| near(c.rect_pt.y1, first[0].rect_pt.y1))
    );
    assert!(
        second
            .iter()
            .all(|c| near(c.rect_pt.y1, second[0].rect_pt.y1))
    );
    assert!(second[0].rect_pt.y0 >= first[0].rect_pt.y1);
    for line in [first, second] {
        let DisplayCmd::DrawStyledGlyphRun {
            origin, max_width, ..
        } = &scene.display_list[line[0].stream_op_index]
        else {
            panic!("search hit does not point to a drawn label");
        };
        assert!(near(
            (line[0].rect_pt.x0 + line.last().unwrap().rect_pt.x1) / 2.0,
            origin.0
        ));
        assert!(line.last().unwrap().rect_pt.x1 - line[0].rect_pt.x0 <= max_width.unwrap() + 0.05);
    }
}

#[test]
fn architecture_keeps_nested_containers_behind_their_diagram_objects() {
    let document = open(include_bytes!("fixtures/06-grouped-architecture.vsdx"));
    let scene = document.interpret_page_scene(0).unwrap();
    for label in [
        "Streaming Platform",
        "Analytical Storage",
        "Stream Workers",
        "Session Enricher",
    ] {
        assert_label(&scene, label);
    }
    let painted_box = |color| {
        scene
            .display_list
            .iter()
            .enumerate()
            .find_map(|(index, cmd)| match cmd {
                DisplayCmd::DrawPath {
                    bounds,
                    fill_rgba: Some(fill),
                    ..
                } if *fill == color => Some((index, *bounds)),
                _ => None,
            })
            .unwrap()
    };
    let (platform_order, platform) = painted_box([239, 246, 255, 255]);
    let (storage_order, storage) = painted_box([240, 253, 244, 255]);
    let (store_order, store) = painted_box([225, 213, 231, 255]);
    assert!(platform_order < storage_order && storage_order < store_order);
    for (outer, inner) in [(platform, storage), (storage, store)] {
        assert!(inner.x0 > outer.x0 && inner.x1 < outer.x1);
        assert!(inner.y0 > outer.y0 && inner.y1 < outer.y1);
    }
}
