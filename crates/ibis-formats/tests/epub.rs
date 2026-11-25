use ibis_formats::{
    epub,
    ibis_ooxml::zip::{EntryMeta, Writer},
};

fn book(fixed: bool) -> Vec<u8> {
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
        r##"<html xmlns:epub="http://www.idpf.org/2007/ops"><body><h1 id="one">第一章</h1>
        <p><ruby>山路<rt>やまみち</rt></ruby>の脚注<a href="two.xhtml#note" epub:type="noteref">1</a>。</p>
        <p><img src="pic%20one.png" alt="検証画像"/></p><p><img src="https://example.invalid/remote.png" alt="外部画像"/></p>
        </body></html>"##,
    );
    add(
        "book/two.xhtml",
        r#"<html xmlns:epub="http://www.idpf.org/2007/ops"><body><h1 id="two">第二章</h1>
        <aside id="note" epub:type="footnote"><p>脚注の本文。</p></aside></body></html>"#,
    );
    let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD,
        "iVBORw0KGgoAAAANSUhEUgAAAAQAAAADCAIAAAA7ljmRAAAAE0lEQVR4nGNkYPjPAANMcBY6BwAkeAEF0tyqqAAAAABJRU5ErkJggg==").unwrap();
    zip.add("book/pic one.png", EntryMeta::new(), &png).unwrap();
    zip.finish().unwrap()
}

#[test]
fn opens_reflowable_epub_with_editable_ruby_images_and_footnotes() {
    let bytes = book(false);
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
    assert!(xml_part("word/footnotes.xml").contains("脚注の本文。"));
}

#[test]
fn rejects_fixed_layout() {
    assert!(epub::to_markdown(&book(true)).is_err());
}
