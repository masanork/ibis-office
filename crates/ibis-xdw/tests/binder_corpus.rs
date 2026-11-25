use ibis_xdw::XdwContainer;

// Download the DocuWorks 8–10 test binders from CodeLibs fess-testdata and
// save them as ver8.xbd, ver9.xbd and ver10.xbd in XBD_CORPUS_DIR.
#[test]
#[ignore = "requires the public CodeLibs binders in XBD_CORPUS_DIR"]
fn preserves_binder_document_order_pages_and_selection() {
    let root = std::path::PathBuf::from(std::env::var_os("XBD_CORPUS_DIR").unwrap());
    for (filename, names) in [
        ("ver8.xbd", ["test_data8-2", "test_data8-2"]),
        ("ver9.xbd", ["test_xdw_ver9-1", "test_xdw_ver9-2"]),
        ("ver10.xbd", ["test_xdw_ver10-1", "test_xdw_ver10-2"]),
    ] {
        let file =
            XdwContainer::from_bytes(std::fs::read(root.join(filename)).unwrap().into()).unwrap();
        let documents = file.binder_documents().unwrap();
        assert_eq!(documents.len(), 2, "{filename}");
        assert_eq!(file.page_infos().len(), 2, "{filename}");
        for (index, (document, name)) in documents.iter().zip(names).enumerate() {
            assert_eq!(document.name, name, "{filename}");
            assert_eq!(document.pages, index..index + 1, "{filename}");
            let scene = file.interpret_page_scene(index).unwrap();
            assert!(!scene.display_list.is_empty(), "{filename}");
            let text = &scene.text_index;
            assert!(!text.chars.is_empty(), "{filename}");
            assert_eq!(
                text.text_between(0, text.chars.len() - 1),
                "Lorem ipsum. 吾輩は猫である。",
                "{filename}"
            );
            let glyph = text.chars[0].rect_pt;
            assert_eq!(
                text.hit((glyph.x0 + glyph.x1) / 2.0, (glyph.y0 + glyph.y1) / 2.0),
                Some(0),
                "{filename}"
            );
        }
    }
}

// Save ezpzfile-xdw's tests/data/dw-binder.bin as sample.xbd in XBD_CORPUS_DIR.
#[test]
#[ignore = "requires the public marker binder in XBD_CORPUS_DIR"]
fn renders_freehand_marker_in_second_document() {
    use ibis_pdf::{DisplayCmd, scene::PathCmd};

    let root = std::path::PathBuf::from(std::env::var_os("XBD_CORPUS_DIR").unwrap());
    let bytes = std::fs::read(root.join("sample.xbd")).unwrap();
    let file = XdwContainer::from_bytes(bytes.into()).unwrap();
    let documents = file.binder_documents().unwrap();
    assert_eq!(documents[0].name, "blank");
    assert_eq!(documents[1].name, "marker");
    assert_eq!(documents[1].pages, 1..2);
    assert!(
        file.interpret_page_scene(0)
            .unwrap()
            .display_list
            .is_empty()
    );
    let marker = file.interpret_page_scene(1).unwrap();
    let quadrilaterals = marker
        .display_list
        .iter()
        .filter(|cmd| {
            matches!(cmd,
                DisplayCmd::DrawPath { commands, fill_rgba: Some([0, 0, 0, 255]), .. }
                    if matches!(commands.as_slice(), [
                        PathCmd::MoveTo(..), PathCmd::LineTo(..), PathCmd::LineTo(..),
                        PathCmd::LineTo(..), PathCmd::Close
                    ])
            )
        })
        .count();
    assert_eq!(quadrilaterals, 4);
}
