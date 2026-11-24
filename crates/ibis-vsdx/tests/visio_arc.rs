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

#[test]
#[ignore = "requires the published Data Warehouse & Analytics Platform VSDX"]
fn reconnects_a_circular_line_jump_in_an_unmodified_visio_drawing() {
    use ibis_ooxml::zip::Archive;
    use ibis_vsdx::geometry_edit::{Endpoint, GeometryEdit};

    let input = PathBuf::from(std::env::var_os("VSDX_VISIO_GLUED_ARC_SAMPLE").unwrap());
    let original = std::fs::read(&input).unwrap();
    let doc = VsdxContainer::from_bytes(original.clone().into()).unwrap();
    let start = (4.900646825681039_f32 * 72.0, 3.960531922441815_f32 * 72.0);
    let curve = |scene: &ibis_pdf::PageScene| {
        let start_y = scene.info.crop_box.height() - start.1;
        scene.display_list.iter().find_map(|cmd| {
            let DisplayCmd::DrawPath { commands, .. } = cmd else {
                return None;
            };
            let Some(PathCmd::MoveTo(x, y)) = commands.first() else {
                return None;
            };
            if (x - start.0).abs() > 0.02 || (y - start_y).abs() > 0.02 {
                return None;
            }
            let controls: Vec<_> = commands
                .iter()
                .filter_map(|cmd| match *cmd {
                    PathCmd::CubicTo(a, b, c, d, e, f) => Some([a, b, c, d, e, f]),
                    _ => None,
                })
                .collect();
            let Some(PathCmd::LineTo(end_x, end_y)) = commands.last() else {
                return None;
            };
            (!controls.is_empty()).then_some((controls, *end_x, *end_y, start_y))
        })
    };
    let (page, before) = (0..doc.page_infos().len())
        .find_map(|page| curve(&doc.interpret_page_scene(page).ok()?).map(|route| (page, route)))
        .expect("Visio arc in the original drawing");

    let edited = doc
        .edit_geometry(
            page,
            GeometryEdit::Reconnect {
                connector: 614,
                endpoint: Endpoint::End,
                target: 253,
            },
        )
        .unwrap();
    let output = std::env::var_os("VSDX_VISIO_GLUED_ARC_OUTPUT").map(PathBuf::from);
    let temp = std::env::temp_dir().join(format!("ibis-native-arc-{}.vsdx", std::process::id()));
    let path = output.as_ref().unwrap_or(&temp);
    assert_ne!(input.canonicalize().ok(), path.canonicalize().ok());
    std::fs::write(path, edited).unwrap();
    let saved = std::fs::read(path).unwrap();
    if output.is_none() {
        std::fs::remove_file(path).unwrap();
    }
    assert_eq!(std::fs::read(input).unwrap(), original);
    let reopened = VsdxContainer::from_bytes(saved.clone().into()).unwrap();
    let after = curve(&reopened.interpret_page_scene(page).unwrap()).unwrap();
    assert!((after.1 - 8.910371699806225_f32 * 72.0).abs() < 0.02);
    assert!((after.2 - before.2).abs() < 0.02);
    let scale = (after.1 - start.0) / (before.1 - start.0);
    assert_eq!(before.0.len(), after.0.len());
    for (a, b) in before.0.iter().zip(&after.0) {
        for i in 0..6 {
            let origin = if i % 2 == 0 { start.0 } else { before.3 };
            assert!((b[i] - (origin + (a[i] - origin) * scale)).abs() < 0.04);
        }
    }

    let before = Archive::new(original.into()).unwrap();
    let after = Archive::new(saved.into()).unwrap();
    let mut changed = 0;
    for entry in before.entries() {
        let copy = after
            .entries()
            .iter()
            .find(|e| e.name == entry.name)
            .unwrap();
        let old = before.read(entry).unwrap();
        let new = after.read(copy).unwrap();
        if old == new {
            continue;
        }
        changed += 1;
        assert!(entry.name.starts_with("visio/pages/page"));
        let new = String::from_utf8(new).unwrap();
        let xml = roxmltree::Document::parse(&new).unwrap();
        assert!(xml.descendants().any(|n| n.has_tag_name("Connect")
            && n.attribute("FromSheet") == Some("614")
            && n.attribute("FromCell") == Some("EndX")
            && n.attribute("ToSheet") == Some("253")));
        assert!(new.contains("_XFTRIGGER(Sheet.253!EventXFMod)"));
    }
    assert_eq!(changed, 1);
}
