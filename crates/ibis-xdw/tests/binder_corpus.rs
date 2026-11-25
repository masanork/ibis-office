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
