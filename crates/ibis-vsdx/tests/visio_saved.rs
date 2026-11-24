// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::PathBuf;

use ibis_pdf::{DisplayCmd, scene::PathCmd};
use ibis_vsdx::VsdxContainer;

#[test]
#[ignore = "download the Apache POI samples to VSDX_VISIO_CORPUS_DIR as documented in fixtures/README.md"]
fn reads_drawings_saved_by_microsoft_visio() {
    let root = PathBuf::from(
        std::env::var_os("VSDX_VISIO_CORPUS_DIR").expect("set VSDX_VISIO_CORPUS_DIR"),
    );
    for (name, (width, height), labels) in [
        (
            "test.vsdx",
            (841.89, 595.276),
            &["This is a test.", "Nothing fancy."][..],
        ),
        (
            "test_text_extraction.vsdx",
            (612.0, 792.0),
            &["Text here", "Text there", "Router here"][..],
        ),
    ] {
        let bytes =
            std::fs::read(root.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
        let document = VsdxContainer::from_bytes(bytes.into())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(document.page_infos().len(), 1, "{name}");
        let scene = document.interpret_page_scene(0).unwrap();
        assert!((scene.info.crop_box.width() - width).abs() < 0.05, "{name}");
        assert!(
            (scene.info.crop_box.height() - height).abs() < 0.05,
            "{name}"
        );
        assert!(
            scene
                .display_list
                .iter()
                .any(|cmd| matches!(cmd, DisplayCmd::DrawPath { .. })),
            "{name}"
        );
        if name == "test.vsdx" {
            // Visio's embedded thumbnail shows a white body and a blue title band.
            assert!(
                scene.display_list.iter().any(|cmd| matches!(cmd,
                    DisplayCmd::DrawPath { bounds, fill_rgba: Some([255, 255, 255, 255]), .. }
                    if bounds.width() > 350.0 && bounds.height() > 200.0
                )),
                "the large container keeps the white fill from Visio's thumbnail"
            );
            assert!(
                scene.display_list.iter().any(|cmd| matches!(
                    cmd,
                    DisplayCmd::DrawPath {
                        fill_rgba: Some([91, 155, 213, 255]),
                        ..
                    }
                )),
                "the star inherits the theme's blue fill"
            );
        }
        if name == "test_text_extraction.vsdx" {
            assert!(scene.display_list.iter().any(|cmd| {
                matches!(cmd, DisplayCmd::DrawPath { bounds, commands, .. }
                    if bounds.y1 < 130.0 && commands.iter().any(|cmd| matches!(cmd, PathCmd::CubicTo(..))))
            }), "missing inherited curved outline of the router");
            assert!(
                scene.display_list.iter().any(|cmd| matches!(cmd,
                    DisplayCmd::DrawStyledGlyphRun { text, font_size, rgba: [77, 30, 26, 255], .. }
                    if text == "Router here" && (*font_size - 10.0).abs() < 0.05
                )),
                "router's inherited label must fit on one line"
            );
        }
        let text = scene.text_index.plain_text();
        for label in labels {
            assert!(text.contains(label), "{name}: {label}");
        }
        let editable = document.editable_labels(0).unwrap();
        let label = editable
            .iter()
            .find(|label| label.text == labels[0])
            .unwrap_or_else(|| panic!("{name}: no editable label for {}", labels[0]));
        let updated = document
            .replace_label(0, label.shape_id, "Updated <Visio> & text")
            .unwrap();
        let reopened = VsdxContainer::from_bytes(updated.into()).unwrap();
        let updated_scene = reopened.interpret_page_scene(0).unwrap();
        assert_eq!(updated_scene.info.crop_box, scene.info.crop_box);
        let updated_text = updated_scene.text_index.plain_text();
        assert!(
            updated_text.contains("Updated <Visio> & text"),
            "{name}: {updated_text}"
        );
        assert!(!updated_text.contains(labels[0]), "{name}");
        for label in &labels[1..] {
            assert!(updated_text.contains(label), "{name}: {label}");
        }
    }
}
