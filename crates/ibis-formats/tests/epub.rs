use ibis_formats::{
    epub,
    ibis_ooxml::zip::{EntryMeta, Writer},
};

fn book(fixed: bool, mixed: bool) -> Vec<u8> {
    let mut zip = Writer::new();
    let mime = b"application/epub+zip";
    zip.add_raw(
        "mimetype",
        EntryMeta::new(),
        0,
        ibis_formats::ibis_ooxml::zip::crc32(mime),
        mime.len(),
        mime,
    )
    .unwrap();
    let mut add = |name, text: &str| zip.add(name, EntryMeta::new(), text.as_bytes()).unwrap();
    add(
        "META-INF/container.xml",
        r#"<container><rootfiles><rootfile full-path="book/content.opf"/></rootfiles></container>"#,
    );
    let layout = if fixed {
        r#"<meta property="rendition:layout">pre-paginated</meta>"#
    } else {
        ""
    };
    add(
        "book/content.opf",
        &format!(
            r#"<package version="3.0"><metadata><title>試験</title>{layout}</metadata><manifest>
        <item id="two" href="two.xhtml" media-type="application/xhtml+xml"/>
        <item id="nav" href="nav.xhtml" properties="nav" media-type="application/xhtml+xml"/>
        <item id="one" href="one.xhtml" media-type="application/xhtml+xml"/>
        </manifest><spine><itemref idref="one"/><itemref idref="two"/></spine></package>"#
        ),
    );
    add(
        "book/nav.xhtml",
        r#"<html xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="one.xhtml#one">第一章へのリンク</a></li><li><a href="two.xhtml#two">第二章へのリンク</a></li></ol></nav></body></html>"#,
    );
    add(
        "book/one.xhtml",
        r##"<html xmlns:epub="http://www.idpf.org/2007/ops"><body style="writing-mode: vertical-rl"><h1 id="one">第一章</h1>
        <p><ruby>山路<rt>やまみち</rt></ruby>の脚注<a href="two.xhtml#note" epub:type="noteref">1</a>。</p>
        <p><span id="inline">本文内の目印</span> <a id="old" class="note-d" href="two.xhtml#oldnote">[2]</a></p>
        <table id="table"><tr><th colspan="2">見出し</th><th>数</th></tr><tr><td rowspan="2">分類</td><td id="cell">甲</td><td>1</td></tr><tr><td>乙<table><tr><td>内側</td><td><ruby>山路<rt>やまみち</rt></ruby></td></tr></table></td><td>2</td></tr></table>
        <p><img src="pic%20one.png" alt="検証画像" width="12" height="9"/></p><p><img src="https://example.invalid/remote.png" alt="外部画像"/></p>
        </body></html>"##,
    );
    let second_chapter = r##"<html xmlns:epub="http://www.idpf.org/2007/ops"><body><h1 id="two">第二章</h1>
        <p><a href="one.xhtml#inline">本文内へのリンク</a> <a href="one.xhtml#cell">セルへのリンク</a></p>
        <aside id="note" epub:type="footnote"><p>脚注の本文。</p></aside>
        <ul><li id="oldnote"><a href="one.xhtml#old">[2]</a>旧形式の脚注。</li><li>普通の項目</li></ul></body></html>"##;
    let second_chapter = if mixed {
        second_chapter.replace("<body>", "<body style=\"writing-mode:horizontal-tb\">")
    } else {
        second_chapter.to_owned()
    };
    add("book/two.xhtml", &second_chapter);
    let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD,
        "iVBORw0KGgoAAAANSUhEUgAAAAQAAAADCAIAAAA7ljmRAAAAE0lEQVR4nGNkYPjPAANMcBY6BwAkeAEF0tyqqAAAAABJRU5ErkJggg==").unwrap();
    zip.add("book/pic one.png", EntryMeta::new(), &png).unwrap();
    zip.finish().unwrap()
}

#[test]
fn opens_reflowable_epub_with_editable_ruby_images_and_footnotes() {
    let bytes = book(false, false);
    assert!(epub::is_epub(&bytes));
    let md = epub::to_markdown(&bytes).unwrap();
    assert!(md.find("# 第一章").unwrap() < md.find("# 第二章").unwrap());
    assert!(md.contains("[第一章へのリンク](#epub2)"));
    assert!(md.contains("{山路|やまみち}の脚注[^1]"));
    assert!(md.contains("![検証画像](data:image/png;base64,"));
    assert!(md.contains("外部画像"));
    assert!(!md.contains("example.invalid"));
    let opened = ibis_formats::open(bytes).unwrap();
    let docx = opened
        .source
        .save(&opened.presentation, Default::default())
        .unwrap();
    let archive = ibis_formats::ibis_ooxml::zip::Archive::new(docx.into()).unwrap();
    let xml_part = |path| {
        let entry = archive
            .entries()
            .iter()
            .find(|entry| entry.name == path)
            .unwrap();
        String::from_utf8(archive.read(entry).unwrap()).unwrap()
    };
    let main = xml_part("word/document.xml");
    assert!(main.contains("<w:ruby>"));
    assert!(main.contains("w:anchor=\"epub2\""));
    assert!(main.contains("w:name=\"epub2\""));
    assert!(main.contains("<w:drawing"));
    assert!(main.contains("cx=\"114300\""));
    assert!(main.contains("cy=\"85725\""));
    assert!(md.contains("{width=12px height=9px}"));
    assert!(main.contains("w:name=\"epub3\""));
    assert!(main.contains("w:anchor=\"epub3\""));
    let tree = roxmltree::Document::parse(&main).unwrap();
    let table = tree.descendants().find(|n| n.has_tag_name("tbl")).unwrap();
    let cell = table
        .descendants()
        .find(|n| {
            n.has_tag_name("tc")
                && n.descendants()
                    .any(|n| n.is_text() && n.text() == Some("甲"))
        })
        .unwrap();
    let bookmark = cell
        .descendants()
        .find(|n| n.has_tag_name("bookmarkStart"))
        .unwrap();
    let name = bookmark
        .attributes()
        .find(|a| a.name() == "name")
        .unwrap()
        .value();
    assert!(main.contains(&format!("w:anchor=\"{name}\"")));
    assert!(main.contains("<w:tbl>"));
    assert!(main.contains("w:gridSpan"));
    assert_eq!(
        table
            .descendants()
            .filter(|n| n.has_tag_name("tbl"))
            .count(),
        2
    );
    assert!(main.contains("内側"));
    assert!(main.contains("w:vMerge"));
    assert!(main.contains("tbRl"));
    assert!(md.contains("direction: vertical"));
    let footnotes = xml_part("word/footnotes.xml");
    assert!(footnotes.contains("旧形式の脚注。"));
    assert!(!main.contains("旧形式の脚注。"));
    assert!(xml_part("word/footnotes.xml").contains("脚注の本文。"));
}

#[test]
fn rejects_fixed_layout() {
    assert!(epub::to_markdown(&book(true, false)).is_err());
}

#[test]
fn mixed_writing_directions_survive_markdown_and_docx_round_trip() {
    let bytes = book(false, true);
    let md = epub::to_markdown(&bytes).unwrap();
    assert!(md.contains(r#"::: section {"break":"next-page","direction":"vertical"}"#));
    let docx = epub::to_docx(&bytes).unwrap();
    let exported = ibis_formats::markdown::export_docx_to_markdown(&docx).unwrap();
    assert!(exported.contains(r#""direction":"vertical""#));
    assert!(exported.contains("<!-- docx-bookmark:"));
    assert!(exported.contains("<!-- nested-table:"));
    let reopened =
        ibis_formats::markdown::import_markdown_to_docx_bytes(exported.as_bytes()).unwrap();
    let opened = ibis_formats::open(reopened).unwrap();
    let flow = opened.presentation.flow.unwrap();
    assert_eq!(flow.page.direction, ibis_doc_model::CellDirection::LrTb);
    assert!(flow.body.iter().any(|block| matches!(block, ibis_doc_model::Block::Paragraph(p)
        if p.section.as_ref().is_some_and(|section| section.page.direction == ibis_doc_model::CellDirection::TbRl))));
}
