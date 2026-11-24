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
        scene
            .display_list
            .iter()
            .any(|cmd| { matches!(cmd, DisplayCmd::DrawGlyphRun { text, .. } if text == label) }),
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
    assert_label(&scene, "Within 30 days?");
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
        assert!(
            scene
                .display_list
                .iter()
                .any(|cmd| matches!(cmd, DisplayCmd::DrawPath { .. }))
        );
    }
}
