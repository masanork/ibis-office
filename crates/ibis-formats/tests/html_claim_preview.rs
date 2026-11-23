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
        let run_review = || {
            std::process::Command::new(env!("CARGO_BIN_EXE_docx_form"))
                .current_dir(dir.path())
                .args([
                    "review-html-claims",
                    "layout.html",
                    "form.json",
                    "record.json",
                    "claims.json",
                    "review.html",
                ])
                .output()
                .unwrap()
        };
        let output = run_review();
        assert!(output.status.success(), "{output:?}");
        let page = std::fs::read_to_string(dir.path().join("review.html")).unwrap();
        assert!(page.contains("sandbox srcdoc=\"&lt;p&gt;安全研修 令和8年4月1日 00123"));
        assert!(page.contains("note</code>（3文字、上限2文字）"));
        let encoded = page
            .split_once("data:application/json;base64,")
            .unwrap()
            .1
            .split('"')
            .next()
            .unwrap();
        use base64::{Engine, engine::general_purpose::STANDARD};
        let downloaded: serde_json::Value =
            serde_json::from_slice(&STANDARD.decode(encoded).unwrap()).unwrap();
        assert_eq!(downloaded, preview["claims"]);
        assert!(!run_review().status.success());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("review.html")).unwrap(),
            page
        );
    }

    let error = preview_html_claims("{{serial}}", &definition, &record, &mapping).unwrap_err();
    assert!(error.to_string().contains("absent from the print layout"));
    mapping["form"]["version"] = json!(2);
    let error = preview_html_claims(layout, &definition, &record, &mapping).unwrap_err();
    assert!(error.to_string().contains("different form ID or version"));
    mapping["form"]["version"] = json!(1);
    mapping["fieldLabels"] = json!({"serial": "管理番号"});
    assert_eq!(
        preview_html_claims(layout, &definition, &record, &mapping).unwrap(),
        preview
    );
    mapping["fieldLabels"] = json!({"unknown": "管理番号"});
    let error = preview_html_claims(layout, &definition, &record, &mapping).unwrap_err();
    assert!(error.to_string().contains("Invalid field label: unknown"));
    mapping.as_object_mut().unwrap().remove("fieldLabels");
    definition["fields"]["serial"] = json!({"type": "text", "maxChars": 3});
    let error = preview_html_claims(layout, &definition, &record, &mapping).unwrap_err();
    assert!(error.to_string().contains("serial (5 characters, limit 3)"));
    mapping["fieldLabels"] = json!({"serial": "管理番号"});
    let error = preview_html_claims(layout, &definition, &record, &mapping).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("管理番号 (serial) (5 characters, limit 3)")
    );
    record["data"]["serial"] = json!("");
    let error = preview_html_claims(layout, &definition, &record, &mapping).unwrap_err();
    assert!(error.to_string().contains("blank: 管理番号 (serial)"));
    record["data"].as_object_mut().unwrap().remove("serial");
    assert!(preview_html_claims(layout, &definition, &record, &mapping).is_err());
}
