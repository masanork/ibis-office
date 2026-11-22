// SPDX-License-Identifier: AGPL-3.0-only

#![cfg(not(target_family = "wasm"))]

use std::{path::Path, process::Command, sync::Arc};

use ibis_formats::form_template::pdf;
use ibis_pdf::{CosDict, CosObj, CosString, PdfContainer, SaveMode, StreamData, scene};
use krilla::{
    Document, SerializeSettings,
    configure::{Archival, ConfigurationBuilder},
    geom::Point,
    text::{Font, TextDirection},
};
use serde_json::json;
use sha2::{Digest, Sha256};

const FONT: &[u8] = include_bytes!("../../ibis-office-app/fonts/IBMPlexSans-Regular.ttf");

fn name(bytes: &[u8]) -> CosObj {
    CosObj::Name(bytes.to_vec())
}

fn dict<const N: usize>(items: [(&[u8], CosObj); N]) -> CosDict {
    let mut dict = CosDict::new();
    for (key, value) in items {
        dict.set(key, value);
    }
    dict
}

fn get_dict(pdf: &mut PdfContainer, object: &CosObj) -> CosDict {
    pdf.deref(object).unwrap().as_dict().unwrap().clone()
}

fn rectangle(x: i64, y: i64, width: i64, height: i64) -> CosObj {
    CosObj::Array([x, y, x + width, y + height].map(CosObj::Int).to_vec())
}

fn template() -> Vec<u8> {
    let configuration = ConfigurationBuilder::new()
        .with_archival_validator(Archival::A2_B)
        .finish()
        .unwrap();
    let mut generated = Document::new_with(SerializeSettings {
        configuration,
        ..Default::default()
    });
    let font = Font::new(Arc::new(FONT.to_vec()).into(), 0).unwrap();
    for _ in 0..2 {
        let mut page = generated.start_page();
        let mut surface = page.surface();
        surface.draw_text(
            Point::from_xy(20.0, 20.0),
            font.clone(),
            11.0,
            "Template",
            false,
            TextDirection::Auto,
        );
        surface.finish();
        page.finish();
    }
    let mut pdf = PdfContainer::from_bytes(generated.finish().unwrap()).unwrap();
    let pages = scene::collect_pages(&mut pdf).unwrap();
    let page = get_dict(&mut pdf, &CosObj::Ref(pages[0].object_id));
    let resources = page.get(b"Resources").unwrap().clone();
    let resource_dict = get_dict(&mut pdf, &resources);
    let fonts = get_dict(&mut pdf, resource_dict.get(b"Font").unwrap());
    let font_name = std::str::from_utf8(&fonts.entries()[0].0).unwrap();
    let da = CosObj::Str(CosString::Literal(
        format!("/{font_name} 11 Tf 0 g").into_bytes(),
    ));
    let appearance = pdf.alloc_object(CosObj::Stream {
        dict: dict([
            (b"Type", name(b"XObject")),
            (b"Subtype", name(b"Form")),
            (b"BBox", rectangle(0, 0, 212, 40)),
            (b"Resources", CosObj::Dict(CosDict::new())),
        ]),
        data: StreamData::Owned {
            bytes: b"q Q\n".to_vec(),
            encoded: false,
        },
    });
    let root_field = pdf.alloc_object(CosObj::Null);
    let field = pdf.alloc_object(CosObj::Null);
    let mut widgets = Vec::new();
    for (index, page) in pages.into_iter().enumerate() {
        let mk = dict([
            (b"BG", CosObj::Array(vec![CosObj::Real(0.95)])),
            (b"BC", CosObj::Array(vec![CosObj::Int(0)])),
        ]);
        let widget = pdf.alloc_object(CosObj::Dict(dict([
            (b"Type", name(b"Annot")),
            (b"Subtype", name(b"Widget")),
            (b"Parent", CosObj::Ref(field)),
            (b"P", CosObj::Ref(page.object_id)),
            (b"Rect", rectangle(20, 660, 212 - index as i64 * 10, 40)),
            (b"F", CosObj::Int(4)),
            (b"AP", CosObj::Dict(dict([(b"N", CosObj::Ref(appearance))]))),
            (b"MK", CosObj::Dict(mk)),
        ])));
        let mut page_dict = get_dict(&mut pdf, &CosObj::Ref(page.object_id));
        page_dict.set(b"Annots", CosObj::Array(vec![CosObj::Ref(widget)]));
        pdf.set_object(page.object_id, CosObj::Dict(page_dict));
        widgets.push(CosObj::Ref(widget));
    }
    pdf.set_object(
        root_field,
        CosObj::Dict(dict([
            (b"T", CosObj::Str(CosString::Literal(b"student".to_vec()))),
            (b"FT", name(b"Tx")),
            (b"Ff", CosObj::Int(4096)),
            (b"Kids", CosObj::Array(vec![CosObj::Ref(field)])),
        ])),
    );
    pdf.set_object(
        field,
        CosObj::Dict(dict([
            (b"T", CosObj::Str(CosString::Literal(b"name".to_vec()))),
            (b"Parent", CosObj::Ref(root_field)),
            (b"Q", CosObj::Int(1)),
            (b"Kids", CosObj::Array(widgets)),
        ])),
    );
    let (catalog_id, mut catalog) = pdf.catalog().unwrap();
    catalog.set(
        b"AcroForm",
        CosObj::Dict(dict([
            (b"Fields", CosObj::Array(vec![CosObj::Ref(root_field)])),
            (b"DA", da),
            (b"DR", resources),
        ])),
    );
    pdf.set_object(catalog_id, CosObj::Dict(catalog));
    pdf.save(SaveMode::Incremental).unwrap()
}

#[test]
fn inspect_counts_widgets_under_an_inherited_text_field() {
    assert_eq!(pdf::inspect(template()).unwrap()["student.name"], 2);
}

#[test]
fn cli_fills_each_widget_with_a_validated_appearance() {
    let Some(validator) = std::env::var_os("IBIS_TEST_VERAPDF") else {
        eprintln!("Set IBIS_TEST_VERAPDF to test end-to-end PDF/A conformance");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("template.pdf");
    let definition = directory.path().join("form.json");
    let record = directory.path().join("record.yaml");
    let font = directory.path().join("font.ttf");
    let output = directory.path().join("filled.pdf");
    let template = template();
    std::fs::write(&input, &template).unwrap();
    std::fs::write(
        &definition,
        json!({
            "form": {"id": "school", "version": 1},
            "templateSha256": format!("{:x}", Sha256::digest(&template)),
            "fields": {"student.name": {"type": "text", "maxChars": 8}}
        })
        .to_string(),
    )
    .unwrap();
    let sample = if let Some(path) = std::env::var_os("IBIS_TEST_JAPANESE_FONT") {
        std::fs::copy(path, &font).unwrap();
        "架空中学校"
    } else {
        std::fs::write(&font, FONT).unwrap();
        "Academy"
    };
    let expected = format!("{sample}\nA123456")
        .chars()
        .take(8)
        .collect::<String>();
    std::fs::write(&record, format!("form:\n  id: school\n  version: 1\ndata:\n  student:\n    name: \"{sample}\\nA123456\"\n")).unwrap();

    let run = |mode, output: Option<&Path>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_docx_form"));
        command
            .arg(mode)
            .args([&input, &definition, &record, &font])
            .arg(&validator);
        if let Some(output) = output {
            command.arg(output);
        }
        command.output().unwrap()
    };
    let result = run("check-pdf", None);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["truncations"][0]["fieldId"], "student.name");

    let result = run("fill-pdf", Some(&output));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let filled = std::fs::read(&output).unwrap();
    assert!(filled.starts_with(&template));
    let mut pdf = PdfContainer::from_bytes(filled.clone()).unwrap();
    let (_, catalog) = pdf.catalog().unwrap();
    let acro = get_dict(&mut pdf, catalog.get(b"AcroForm").unwrap());
    let root = get_dict(&mut pdf, &acro.get_array(b"Fields").unwrap()[0]);
    let field = get_dict(&mut pdf, &root.get_array(b"Kids").unwrap()[0]);
    let Some(CosObj::Str(value)) = field.get(b"V") else {
        panic!("Missing field value")
    };
    assert_eq!(value.to_text_lossy(), expected);
    let mut appearance_ids = Vec::new();
    for widget in field.get_array(b"Kids").unwrap() {
        let widget = get_dict(&mut pdf, widget);
        let ap = get_dict(&mut pdf, widget.get(b"AP").unwrap());
        let id = ap.get_ref(b"N").unwrap();
        let CosObj::Stream { dict, data } = pdf.resolve(id).unwrap() else {
            panic!("Missing widget stream")
        };
        let instructions = pdf.decode_stream(&dict, &data).unwrap();
        assert!(instructions.windows(2).any(|operator| operator == b"BT"));
        assert!(
            instructions
                .windows(7)
                .any(|operator| operator == b"/Tx BMC")
        );
        appearance_ids.push(id);
    }
    assert_ne!(appearance_ids[0], appearance_ids[1]);
    if let Some(executable) = std::env::var_os("IBIS_TEST_PDFTOTEXT") {
        let result = Command::new(executable)
            .arg(&output)
            .arg("-")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let rendered = String::from_utf8(result.stdout).unwrap();
        assert_eq!(rendered.matches(sample).count(), 2, "{rendered}");
    }

    // Existing files are protected even if generation and validation succeed.
    let result = run("fill-pdf", Some(&output));
    assert!(!result.status.success());
    assert_eq!(std::fs::read(&output).unwrap(), filled);

    std::fs::write(
        &record,
        "form:\n  id: school\n  version: 1\ndata:\n  student:\n    name: \"\"\n",
    )
    .unwrap();
    let result = run("check-pdf", None);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    // Overflow is a preflight error and leaves no output file.
    let mut form: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&definition).unwrap()).unwrap();
    form["fields"]["student.name"]["maxChars"] = json!(400);
    std::fs::write(&definition, form.to_string()).unwrap();
    std::fs::write(
        &record,
        format!(
            "form:\n  id: school\n  version: 1\ndata:\n  student:\n    name: {}\n",
            "A".repeat(200)
        ),
    )
    .unwrap();
    let overflow = directory.path().join("overflow.pdf");
    let result = run("fill-pdf", Some(&overflow));
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("exceeds the widget width"), "{error}");
    assert!(!overflow.exists());
}
