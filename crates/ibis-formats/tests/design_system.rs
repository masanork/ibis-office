// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! End-to-end tests for design-system resolution, DADS CSS token import,
//! and Markdown single-source `.docx` + `.pptx` generation.

#![cfg(not(target_family = "wasm"))]

use std::path::PathBuf;

use ibis_doc_model::{Block, EmuSize, Rgb, ShapeKind};
use ibis_formats::design_system::{
    self, DesignSystemScope, DesignTokens, import_dads_css,
};
use ibis_formats::markdown::{
    export_presentation_to_markdown, open_markdown_presentation_with_design_system_dirs,
    open_markdown_with_design_system_dirs,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/design_systems/dads-excerpt")
}

fn theme_color<'a>(colors: &'a [(String, Rgb)], name: &str) -> Option<Rgb> {
    colors.iter().find(|(k, _)| k == name).map(|(_, rgb)| *rgb)
}

#[test]
fn dads_css_excerpt_imports_to_tokens_and_matches_fixture_theme() {
    let css = std::fs::read_to_string(fixture_dir().join("dads-tokens-excerpt.css"))
        .expect("read DADS CSS excerpt fixture");
    let imported = import_dads_css(&css).expect("import DADS CSS");
    let imported_theme = imported.to_theme().expect("imported theme");

    // Verify real DADS values (Blue-900 = #0017C1, Blue-50 = #E8F1FE) and typography scale.
    assert_eq!(imported_theme.primary, Rgb(0x0017C1));
    assert_eq!(imported_theme.surface_subtle, Rgb(0xE8F1FE));
    assert_eq!(imported_theme.color("accent1"), Some(Rgb(0x0017C1)));
    assert_eq!(imported_theme.color("lt2"), Some(Rgb(0xE8F1FE)));
    assert_eq!(imported_theme.color("dk1"), Some(Rgb(0x1A1A1C)));
    assert_eq!(imported_theme.color("lt1"), Some(Rgb(0xFFFFFF)));
    assert_eq!(imported_theme.chart_colors[0], Rgb(0x0017C1));
    assert_eq!(imported_theme.chart_colors[1], Rgb(0x197A4B));
    assert_eq!(imported_theme.body_font.as_deref(), Some("Noto Sans JP"));
    assert_eq!(imported_theme.body_size_hp, 1200);
    assert_eq!(imported_theme.heading_sizes_hp[0], 2400);
    assert_eq!(imported_theme.slide_title_hp, 2700);
    assert_eq!(imported_theme.slide_cover_title_hp, 4275);

    // YAML round-trip preserves theme mapping.
    let reparsed = DesignTokens::from_yaml(&imported.to_yaml()).expect("reparse YAML");
    assert_eq!(reparsed.to_theme().unwrap(), imported_theme);
}

#[test]
fn resolution_order_project_overrides_system_and_errors_without_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("project");
    let system_dir = tmp.path().join("app-data/ibisOffice/DesignSystems");

    let proj_ds = project_dir.join("DesignSystems/dads-excerpt");
    let sys_ds = system_dir.join("dads-excerpt");
    std::fs::create_dir_all(&proj_ds).unwrap();
    std::fs::create_dir_all(&sys_ds).unwrap();

    // Project has real DADS Blue-900 (#0017C1); system has a different colour (#AA0000).
    for file in ["tokens.yaml", "layouts.yaml", "styles.yaml"] {
        std::fs::copy(fixture_dir().join(file), proj_ds.join(file)).unwrap();
    }
    // Optional base files may exist and must not fail or be applied in this increment.
    std::fs::write(proj_ds.join("base.pptx"), b"unused-base-pptx").unwrap();
    std::fs::write(proj_ds.join("base.docx"), b"unused-base-docx").unwrap();
    std::fs::write(
        sys_ds.join("tokens.yaml"),
        "colors:\n  primary: \"#AA0000\"\n  surface_subtle: \"#FFF0F0\"\n",
    )
    .unwrap();

    let loaded = design_system::load("dads-excerpt", Some(&project_dir), Some(&system_dir))
        .expect("load design system");
    assert_eq!(loaded.scope, DesignSystemScope::Project);
    assert_eq!(loaded.theme().unwrap().primary, Rgb(0x0017C1));
    assert_eq!(loaded.layouts.as_ref().map(|l| l.layouts.len()), Some(3));
    assert!(loaded.styles.as_ref().is_some_and(|s| s.styles.contains_key("heading_1")));
    assert!(loaded.base_pptx.is_some());
    assert!(loaded.base_docx.is_some());

    // Unresolved design system produces a clear error listing both searched paths for DOCX and PPTX.
    let missing_md = b"---\ndesign_system: does-not-exist\n---\n\n# Title\n";
    let docx_err = open_markdown_with_design_system_dirs(
        missing_md.to_vec(),
        Some(&project_dir),
        Some(&system_dir),
    )
    .err()
    .expect("expected missing design system error for docx")
    .to_string();
    assert!(docx_err.contains("does-not-exist"), "{docx_err}");
    assert!(
        docx_err.contains(&project_dir.join("DesignSystems/does-not-exist").display().to_string()),
        "{docx_err}"
    );
    assert!(
        docx_err.contains(&system_dir.join("does-not-exist").display().to_string()),
        "{docx_err}"
    );

    let pptx_err = open_markdown_presentation_with_design_system_dirs(
        missing_md.to_vec(),
        Some(&project_dir),
        Some(&system_dir),
    )
    .err()
    .expect("expected missing design system error for pptx")
    .to_string();
    assert!(pptx_err.contains("does-not-exist"), "{pptx_err}");
    assert!(
        pptx_err.contains(&project_dir.join("DesignSystems/does-not-exist").display().to_string()),
        "{pptx_err}"
    );
    assert!(
        pptx_err.contains(&system_dir.join("does-not-exist").display().to_string()),
        "{pptx_err}"
    );
}

#[test]
fn single_source_markdown_generates_themed_docx_and_pptx_with_same_basename() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("docs-project");
    let out_dir = tmp.path().join("out");
    let ds_dir = project_dir.join("DesignSystems/dads-custom");
    std::fs::create_dir_all(&ds_dir).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    // Generate tokens.yaml via the DADS CSS importer, adding a 4:3 slide size token to verify slide_size application.
    let css = std::fs::read_to_string(fixture_dir().join("dads-tokens-excerpt.css")).unwrap();
    let mut tokens = import_dads_css(&css).expect("import DADS CSS");
    tokens.slide_size = Some(design_system::SlideSizeToken {
        preset: Some("4:3".to_string()),
        width: None,
        height: None,
    });
    std::fs::write(ds_dir.join("tokens.yaml"), tokens.to_yaml()).unwrap();
    std::fs::copy(fixture_dir().join("layouts.yaml"), ds_dir.join("layouts.yaml")).unwrap();
    std::fs::copy(fixture_dir().join("styles.yaml"), ds_dir.join("styles.yaml")).unwrap();

    let md_path = project_dir.join("report.md");
    let markdown = r#"---
title: "デジタル庁デザインシステム適用テスト"
design_system: dads-custom
---

# 表紙タイトル

導入の説明文です。

<!-- notes: ここは発表者ノートです。 -->

---

# 進捗サマリー

- 項目 1
- 項目 2

```chart
type: column
title: 四半期推移
categories: [Q1, Q2]
series:
- 実績: [12, 24]
```

<!-- notes: スライド2の発表者ノート -->

---

# 指標一覧

| 指標 | 値 |
| --- | --- |
| 達成率 | 95% |
"#;
    std::fs::write(&md_path, markdown).unwrap();

    let stem = md_path.file_stem().unwrap().to_string_lossy().to_string();
    assert_eq!(stem, "report");

    // 1. Export DOCX
    let opened_docx = open_markdown_with_design_system_dirs(
        markdown.as_bytes().to_vec(),
        md_path.parent(),
        None,
    )
    .expect("open markdown as docx");
    let docx_bytes = opened_docx
        .source
        .save(&opened_docx.presentation, Default::default())
        .expect("save docx");
    let docx_path = out_dir.join(format!("{stem}.docx"));
    std::fs::write(&docx_path, &docx_bytes).unwrap();

    // 2. Export PPTX
    let opened_pptx = open_markdown_presentation_with_design_system_dirs(
        markdown.as_bytes().to_vec(),
        md_path.parent(),
        None,
    )
    .expect("open markdown as pptx");
    let pptx_bytes = opened_pptx
        .source
        .save(&opened_pptx.presentation, Default::default())
        .expect("save pptx");
    let pptx_path = out_dir.join(format!("{stem}.pptx"));
    std::fs::write(&pptx_path, &pptx_bytes).unwrap();

    // Same base name for both outputs.
    assert_eq!(docx_path.file_stem(), md_path.file_stem());
    assert_eq!(pptx_path.file_stem(), md_path.file_stem());

    // Verify saved DOCX theme colours, fonts, and styles by reading back the generated package.
    let reread_docx = ibis_formats::read(&docx_bytes).expect("read generated docx");
    assert_eq!(theme_color(&reread_docx.theme_colors, "accent1"), Some(Rgb(0x0017C1)));
    assert_eq!(theme_color(&reread_docx.theme_colors, "accent2"), Some(Rgb(0x197A4B)));
    assert_eq!(theme_color(&reread_docx.theme_colors, "lt2"), Some(Rgb(0xE8F1FE)));
    assert_eq!(theme_color(&reread_docx.theme_colors, "dk1"), Some(Rgb(0x1A1A1C)));
    assert_eq!(theme_color(&reread_docx.theme_colors, "lt1"), Some(Rgb(0xFFFFFF)));
    let docx_fonts = reread_docx.theme_fonts.as_ref().expect("docx theme_fonts");
    assert_eq!(docx_fonts.minor.latin.as_deref(), Some("Noto Sans JP"));
    assert_eq!(docx_fonts.major.east_asian.as_deref(), Some("Noto Sans JP"));

    let flow = reread_docx.flow.as_ref().expect("flow document");
    let h1 = flow
        .styles
        .styles
        .values()
        .find(|s| s.name.eq_ignore_ascii_case("heading 1"))
        .expect("Heading 1 style");
    assert_eq!(h1.run.color, Some(Rgb(0x0017C1)));
    assert_eq!(h1.run.size, Some(2400));
    assert_eq!(h1.run.font.as_deref(), Some("Noto Sans JP"));

    // Confirm Markdown export from generated DOCX preserves frontmatter and content.
    let exported_md = export_presentation_to_markdown(&reread_docx).expect("export docx to md");
    assert!(exported_md.contains("design_system: dads-custom"), "{exported_md}");
    assert!(exported_md.contains("導入の説明文です。"), "{exported_md}");

    // Verify saved PPTX theme colours, fonts, slide size, title/body styling, and speaker notes.
    let reread_pptx = ibis_formats::read(&pptx_bytes).expect("read generated pptx");
    assert_eq!(theme_color(&reread_pptx.theme_colors, "accent1"), Some(Rgb(0x0017C1)));
    assert_eq!(theme_color(&reread_pptx.theme_colors, "accent2"), Some(Rgb(0x197A4B)));
    assert_eq!(theme_color(&reread_pptx.theme_colors, "lt2"), Some(Rgb(0xE8F1FE)));
    assert_eq!(theme_color(&reread_pptx.theme_colors, "dk1"), Some(Rgb(0x1A1A1C)));
    assert_eq!(theme_color(&reread_pptx.theme_colors, "lt1"), Some(Rgb(0xFFFFFF)));
    let pptx_fonts = reread_pptx.theme_fonts.as_ref().expect("pptx theme_fonts");
    assert_eq!(pptx_fonts.minor.latin.as_deref(), Some("Noto Sans JP"));
    assert_eq!(pptx_fonts.major.latin.as_deref(), Some("Noto Sans JP"));
    assert_eq!(reread_pptx.slide_size, EmuSize::new(9_144_000, 6_858_000));
    assert_eq!(reread_pptx.slides.len(), 3);

    // Existing `<!-- notes: ... -->` speaker notes extraction must remain intact.
    assert!(
        reread_pptx.slides[0].notes.contains("ここは発表者ノートです。"),
        "slide 0 notes: {:?}",
        reread_pptx.slides[0].notes
    );
    assert!(
        reread_pptx.slides[1].notes.contains("スライド2の発表者ノート"),
        "slide 1 notes: {:?}",
        reread_pptx.slides[1].notes
    );

    // Verify slide title colour and font from design tokens.
    let title_run = reread_pptx.slides[0]
        .shapes
        .iter()
        .find_map(|sh| match &sh.kind {
            ShapeKind::TextBox(tb) => tb.paragraphs.first().and_then(|p| p.runs.first()),
            ShapeKind::Auto(auto) => auto.text.as_ref().and_then(|tb| tb.paragraphs.first()).and_then(|p| p.runs.first()),
            _ => None,
        })
        .expect("slide 0 title run");
    assert_eq!(title_run.text, "表紙タイトル");
    assert_eq!(title_run.style.color, Some(Rgb(0x0017C1)));
    assert_eq!(title_run.style.font_size, Some(4275));

    // Verify table-cell text on slide 2 receives the same theme treatment as body text.
    let table_cell_run = reread_pptx.slides[2]
        .shapes
        .iter()
        .find_map(|sh| match &sh.kind {
            ShapeKind::Table(table) => table
                .rows
                .get(1)
                .and_then(|row| row.cells.first())
                .and_then(|cell| cell.text.paragraphs.first())
                .and_then(|para| para.runs.first()),
            _ => None,
        })
        .expect("slide 2 table cell run");
    assert_eq!(table_cell_run.text, "達成率");
    assert_eq!(table_cell_run.style.color, Some(Rgb(0x1A1A1C)));
    assert_eq!(table_cell_run.style.font_size, Some(1800));
    assert_eq!(table_cell_run.style.font.as_deref(), Some("Noto Sans JP"));

    // Verify the DOCX body contains the heading paragraph.
    assert!(flow.body.iter().any(|b| matches!(b, Block::Paragraph(p) if p.text() == "表紙タイトル")));
}
