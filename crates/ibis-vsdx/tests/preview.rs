// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::sync::Arc;

use ibis_ooxml::zip::{EntryMeta, Writer};
use ibis_pdf::{DisplayCmd, scene::PathCmd};
use ibis_vsdx::{VsdxContainer, is_vsdx};

const VISIO: &str = "http://schemas.microsoft.com/office/visio/2012/main";
const DOC_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn relationship(kind: &str, target: &str, mode: &str) -> String {
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rId1" Type="http://schemas.microsoft.com/visio/2010/relationships/{kind}" Target="{target}" TargetMode="{mode}"/>
        </Relationships>"#
    )
}

fn drawing(external_page: bool, fill: &str) -> Arc<[u8]> {
    let mut zip = Writer::new();
    let mut add = |name: &str, data: &str| {
        zip.add(name, EntryMeta::new(), data.as_bytes()).unwrap();
    };
    add(
        "[Content_Types].xml",
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/drawing/doc.xml" ContentType="application/vnd.ms-visio.drawing.main+xml"/></Types>"#,
    );
    add(
        "_rels/.rels",
        &relationship("document", "drawing/doc.xml", "Internal"),
    );
    add(
        "drawing/doc.xml",
        &format!(r#"<VisioDocument xmlns="{VISIO}"/>"#),
    );
    add(
        "drawing/_rels/doc.xml.rels",
        &relationship("pages", "sheets/index.xml", "Internal").replace(
            "</Relationships>",
            r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="palette.xml"/></Relationships>"#,
        ),
    );
    add(
        "drawing/palette.xml",
        r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
      <a:themeElements><a:clrScheme><a:accent1><a:srgbClr val="A08040"/></a:accent1></a:clrScheme>
      <a:fmtScheme><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"><a:shade val="50000"/></a:schemeClr></a:solidFill></a:fillStyleLst></a:fmtScheme>
      </a:themeElements></a:theme>"#,
    );
    add(
        "drawing/sheets/index.xml",
        &format!(
            r#"<Pages xmlns="{VISIO}" xmlns:r="{DOC_REL}"><Page ID="1">
        <PageSheet><Cell N="PageWidth" V="8"/><Cell N="PageHeight" V="6"/></PageSheet>
        <Rel r:id="rId1"/></Page></Pages>"#
        ),
    );
    add(
        "drawing/sheets/_rels/index.xml.rels",
        &relationship(
            "page",
            "sheet.xml",
            if external_page {
                "External"
            } else {
                "Internal"
            },
        ),
    );
    add(
        "drawing/sheets/sheet.xml",
        &format!(
            r##"<PageContents xmlns="{VISIO}"><Shapes>
        <Shape ID="1" Type="Group">
          <Cell N="PinX" V="1"/><Cell N="PinY" V="1"/>
          <Shapes><Shape ID="2">
            <Cell N="Width" V="2"/><Cell N="Height" V="1"/>
            <Cell N="FillForegnd" V="{fill}"/><Cell N="QuickStyleFillColor" V="2"/><Cell N="QuickStyleFillMatrix" V="1"/>
            <Section N="Geometry" IX="0">
              <Row T="RelMoveTo"><Cell N="X" V="0"/><Cell N="Y" V="0"/></Row>
              <Row T="RelLineTo"><Cell N="X" V="1"/><Cell N="Y" V="0"/></Row>
              <Row T="RelLineTo"><Cell N="X" V="1"/><Cell N="Y" V="1"/></Row>
              <Row T="RelLineTo"><Cell N="X" V="0"/><Cell N="Y" V="1"/></Row>
              <Row T="RelLineTo"><Cell N="X" V="0"/><Cell N="Y" V="0"/></Row>
            </Section><Text><cp IX="0"/>受付</Text>
          </Shape></Shapes>
        </Shape>
        <Shape ID="3">
          <Cell N="BeginX" V="3"/><Cell N="BeginY" V="1.5"/>
          <Cell N="EndX" V="4"/><Cell N="EndY" V="1.5"/>
        </Shape>
        </Shapes></PageContents>"##
        ),
    );
    zip.finish().unwrap().into()
}

#[test]
fn follows_package_relationships_and_draws_shapes_labels_and_connections() {
    let bytes = drawing(false, "#4080c0");
    assert!(is_vsdx(&bytes));
    let document = VsdxContainer::from_bytes(bytes).unwrap();
    let pages = document.page_infos();
    assert_eq!(pages.len(), 1);
    assert_eq!(
        (pages[0].crop_box.width(), pages[0].crop_box.height()),
        (576.0, 432.0)
    );
    let scene = document.interpret_page_scene(0).unwrap();
    let [
        DisplayCmd::DrawPath {
            bounds,
            fill_rgba,
            commands,
            ..
        },
        DisplayCmd::DrawStyledGlyphRun { text, .. },
        DisplayCmd::DrawPath {
            fill_rgba: connector_fill,
            commands: connector,
            ..
        },
    ] = scene.display_list.as_slice()
    else {
        panic!("expected the group rectangle, label, and connector");
    };
    assert_eq!(
        (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
        (72.0, 288.0, 216.0, 360.0)
    );
    assert_eq!(*fill_rgba, Some([64, 128, 192, 255]));
    assert!(matches!(commands.last(), Some(PathCmd::Close)));
    assert_eq!(text, "受付");
    assert_eq!(
        scene
            .text_index
            .chars
            .iter()
            .map(|c| c.ch)
            .collect::<String>(),
        "受付"
    );
    assert!(connector_fill.is_none());
    assert!(matches!(
        connector.as_slice(),
        [PathCmd::MoveTo(216.0, 324.0), PathCmd::LineTo(288.0, 324.0)]
    ));
}

#[test]
fn rejects_an_external_page_relationship() {
    assert!(VsdxContainer::from_bytes(drawing(true, "#4080c0")).is_err());
    assert!(!is_vsdx(b"PK not a ZIP archive"));
}

#[test]
fn reads_solid_theme_fill_through_its_document_relationship() {
    let document = VsdxContainer::from_bytes(drawing(false, "Themed")).unwrap();
    let scene = document.interpret_page_scene(0).unwrap();
    assert!(matches!(
        scene.display_list.first(),
        Some(DisplayCmd::DrawPath {
            fill_rgba: Some([80, 64, 32, 255]),
            ..
        })
    ));
}
