// SPDX-License-Identifier: AGPL-3.0-only
use ibis_formats::form_template::{fill_html_with_diagnostics, preview_html_claims};
use serde_json::json;

#[test]
fn previews_printed_claims_with_original_types() {
    let form = json!({"id": "training-completion", "version": 1});
    let mut definition = json!({
        "form": form,
        "fields": {
            "course.title": "text",
            "completed": "date-ja",
            "serial": "text",
            "hours": "nonnegative-integer",
            "note": {"type": "text", "maxChars": 2}
        }
    });
    let mut record = json!({
        "form": form,
        "data": {
            "course": {"title": "安全研修"},
            "completed": "2026-04-01",
            "serial": "00123",
            "hours": 3,
            "note": "備考欄"
        }
    });
    let mut mapping = json!({
        "form": form,
        "claims": {"training": "course.title", "completion_date": "completed", "number": "serial", "hours": "hours"}
    });
    let layout = "<p>{{course.title}} {{completed}} {{serial}} {{hours}} {{note}}</p>";
    let preview = preview_html_claims(layout, &definition, &record, &mapping).unwrap();
    let (html, diagnostics) = fill_html_with_diagnostics(layout, &definition, &record).unwrap();
    assert_eq!(html, "<p>安全研修 令和8年4月1日 00123 3 備考</p>");
    assert_eq!(preview["html"], html);
    assert_eq!(diagnostics[0].field_id, "note");
    assert_eq!(preview["truncations"][0]["fieldId"], "note");
    assert_eq!(
        preview["claims"],
        json!({"training": "安全研修", "completion_date": "2026-04-01", "number": "00123", "hours": 3})
    );
    #[cfg(not(target_family = "wasm"))]
    {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("layout.html"), layout).unwrap();
        for (name, value) in [
            ("form.json", &definition),
            ("record.json", &record),
            ("claims.json", &mapping),
        ] {
            std::fs::write(dir.path().join(name), value.to_string()).unwrap();
        }
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_docx_form"))
            .current_dir(dir.path())
            .args([
                "preview-html-claims",
                "layout.html",
                "form.json",
                "record.json",
                "claims.json",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            preview
        );
    }

    let error = preview_html_claims("{{serial}}", &definition, &record, &mapping).unwrap_err();
    assert!(error.to_string().contains("absent from the print layout"));
    mapping["form"]["version"] = json!(2);
    let error = preview_html_claims(layout, &definition, &record, &mapping).unwrap_err();
    assert!(error.to_string().contains("different form ID or version"));
    mapping["form"]["version"] = json!(1);
    definition["fields"]["serial"] = json!({"type": "text", "maxChars": 3});
    let error = preview_html_claims(layout, &definition, &record, &mapping).unwrap_err();
    assert!(error.to_string().contains("truncated"));
    record["data"]["serial"] = json!("");
    let error = preview_html_claims(layout, &definition, &record, &mapping).unwrap_err();
    assert!(error.to_string().contains("blank"));
    record["data"].as_object_mut().unwrap().remove("serial");
    assert!(preview_html_claims(layout, &definition, &record, &mapping).is_err());
}
