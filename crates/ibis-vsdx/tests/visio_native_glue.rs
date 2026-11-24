// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::PathBuf;

use ibis_pdf::{DisplayCmd, scene::PathCmd};
use ibis_vsdx::VsdxContainer;

#[test]
#[ignore = "requires the libvisio testfile3.vsdx sample"]
fn reconnects_a_native_visio_connector_using_master_connection_points() {
    use ibis_ooxml::zip::Archive;
    use ibis_vsdx::geometry_edit::{Endpoint, GeometryEdit};

    let input = PathBuf::from(std::env::var_os("VSDX_VISIO_GLUE_SAMPLE").unwrap());
    let original = std::fs::read(&input).unwrap();
    let document = VsdxContainer::from_bytes(original.clone().into()).unwrap();
    let updated = document
        .edit_geometry(
            0,
            GeometryEdit::Reconnect {
                connector: 17,
                endpoint: Endpoint::End,
                target: 1,
            },
        )
        .unwrap();

    // Keep an optional output for examination in an independent viewer.
    let output = std::env::var_os("VSDX_VISIO_GLUE_OUTPUT").map(PathBuf::from);
    let temporary = std::env::temp_dir().join(format!(
        "ibis-visio-native-glue-{}.vsdx",
        std::process::id()
    ));
    let destination = output.as_ref().unwrap_or(&temporary);
    assert_ne!(input.canonicalize().ok(), destination.canonicalize().ok());
    std::fs::write(destination, updated).unwrap();
    let saved = std::fs::read(destination).unwrap();
    if output.is_none() {
        std::fs::remove_file(destination).unwrap();
    }
    assert_eq!(std::fs::read(&input).unwrap(), original);
    let opened = VsdxContainer::from_bytes(saved.clone().into()).unwrap();
    let scene = opened.interpret_page_scene(0).unwrap();
    let route = scene
        .display_list
        .iter()
        .find_map(|cmd| {
            let DisplayCmd::DrawPath { commands, .. } = cmd else {
                return None;
            };
            match commands.as_slice() {
                [
                    PathCmd::MoveTo(x0, y0),
                    PathCmd::LineTo(..),
                    PathCmd::LineTo(..),
                    PathCmd::LineTo(x1, y1),
                ] => {
                    let page_height = scene.info.crop_box.height();
                    ((x0 - 5.610236 * 72.0).abs() < 0.02
                        && (y0 - (page_height - 4.141732 * 72.0)).abs() < 0.02)
                        .then_some((*x1, *y1))
                }
                _ => None,
            }
        })
        .expect("rendered route starting at its original endpoint");
    assert!((route.0 - 2.2637795 * 72.0).abs() < 0.02, "{route:?}");
    assert!(
        (route.1 - (scene.info.crop_box.height() - 5.2244094 * 72.0)).abs() < 0.02,
        "{route:?}"
    );

    let old = Archive::new(original.into()).unwrap();
    let new = Archive::new(saved.into()).unwrap();
    for entry in old.entries() {
        let replacement = new
            .entries()
            .iter()
            .find(|candidate| candidate.name == entry.name)
            .unwrap();
        if entry.name == "visio/pages/page1.xml" {
            let xml = String::from_utf8(new.read(replacement).unwrap()).unwrap();
            let page = roxmltree::Document::parse(&xml).unwrap();
            assert!(page.descendants().any(|n| n.has_tag_name("Connect")
                && n.attribute("FromSheet") == Some("17")
                && n.attribute("FromCell") == Some("EndX")
                && n.attribute("ToSheet") == Some("1")
                && n.attribute("ToCell") == Some("Connections.X2")));
            let connector = page
                .descendants()
                .find(|n| n.has_tag_name("Shape") && n.attribute("ID") == Some("17"))
                .unwrap();
            for (cell, expected) in [("EndX", 2.263779527559055), ("EndY", 5.224409448818897)] {
                let cell = connector
                    .children()
                    .find(|n| n.attribute("N") == Some(cell))
                    .unwrap();
                assert_eq!(
                    cell.attribute("F"),
                    Some("PAR(PNT(Sheet.1!Connections.X2,Sheet.1!Connections.Y2))")
                );
                assert!(
                    (cell.attribute("V").unwrap().parse::<f64>().unwrap() - expected).abs() < 1e-6
                );
            }
        } else {
            assert_eq!(
                old.read(entry).unwrap(),
                new.read(replacement).unwrap(),
                "{}",
                entry.name
            );
        }
    }
}
