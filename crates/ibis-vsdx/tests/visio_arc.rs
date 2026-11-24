// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::PathBuf;

use ibis_pdf::{DisplayCmd, scene::PathCmd};
use ibis_vsdx::VsdxContainer;

#[test]
#[ignore = "requires the published Aspose modified.vsdx sample"]
fn renders_circular_line_jumps_in_a_published_visio_file() {
    let path = PathBuf::from(std::env::var_os("VSDX_VISIO_ARC_SAMPLE").unwrap());
    let bytes = std::fs::read(path).unwrap();
    let doc = VsdxContainer::from_bytes(bytes.into()).unwrap();
    let mut found = false;
    for page in 0..doc.page_infos().len() {
        let scene = doc.interpret_page_scene(page).unwrap();
        for command in &scene.display_list {
            let DisplayCmd::DrawPath { commands, .. } = command else {
                continue;
            };
            let Some(PathCmd::MoveTo(x, y)) = commands.first() else {
                continue;
            };
            if (*x - 1.69661458 * 72.0).abs() > 0.02 {
                continue;
            }
            let curved: Vec<_> = commands
                .iter()
                .filter_map(|command| match command {
                    PathCmd::CubicTo(_, first_y, _, second_y, end_x, end_y) => {
                        Some([*first_y, *second_y, *end_x, *end_y])
                    }
                    _ => None,
                })
                .collect();
            let endpoints = [1.69661458 + 1.54166667, 1.69661458 + 3.29166667];
            if !endpoints.into_iter().all(|end| {
                curved
                    .iter()
                    .any(|p| (p[2] - end * 72.0).abs() < 0.02 && (p[3] - y).abs() < 0.02)
            }) {
                continue;
            }
            assert!(curved.iter().any(|p| p[0].min(p[1]).min(p[3]) < y - 2.8));
            found = true;
        }
    }
    assert!(found, "two upward semicircular jumps");
}
