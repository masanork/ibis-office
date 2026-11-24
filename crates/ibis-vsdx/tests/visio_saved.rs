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
        if name == "test_text_extraction.vsdx" {
            assert!(scene.display_list.iter().any(|cmd| {
                matches!(cmd, DisplayCmd::DrawPath { bounds, commands, .. }
                    if bounds.y1 < 130.0 && commands.iter().any(|cmd| matches!(cmd, PathCmd::CubicTo(..))))
            }), "missing inherited curved outline of the router");
        }
        let text = scene.text_index.plain_text();
        for label in labels {
            assert!(text.contains(label), "{name}: {label}");
        }
    }
}
