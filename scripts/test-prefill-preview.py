#!/usr/bin/env python3
"""Exercise the read-only Excel prefill plan through the CLI and MCP."""
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from zipfile import ZipFile

build = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug")
fixtures = Path("test/fixtures/structured-preview").resolve()
workbook = fixtures / "site-entry-form.xlsx"
mapping = fixtures / "prefill-sd-jwt.json"
settings = json.loads(mapping.read_text(encoding="utf-8"))
original = workbook.read_bytes()

with subprocess.Popen(
    [str(build / "ibisOffice-mcp"), "--read-only", "--allow-dir", str(fixtures)],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, encoding="utf-8",
) as server:
    def call(name, arguments, error=False):
        request = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                   "params": {"name": name, "arguments": arguments}}
        server.stdin.write(json.dumps(request) + "\n")
        server.stdin.flush()
        response = json.loads(server.stdout.readline())
        if "error" in response:
            assert error, response
            return response
        result = response["result"]
        assert result["isError"] is error, result
        return result["structuredContent"]

    try:
        doc = call("app_open_document", {"path": str(workbook)})["document_id"]
        for format_name, prefix, suffix, number, trust in [
            ("sd-jwt", "", "", None, "not_checked"),
            ("json", "/", "", "00123", "not_applicable"),
            ("xml", "/修了記録/", "[1]/#text[1]", "00123", "not_applicable"),
            ("mdoc", "/org.example.training/", "", None, "not_checked"),
        ]:
            source = fixtures / f"training.{format_name}"
            mappings = [dict(entry) for entry in settings["mappings"]]
            for entry in mappings:
                if "source_path" in entry:
                    entry["source_path"] = prefix + entry["source_path"] + suffix
            args = {"document_id": doc, "sheet": settings["sheet"], "format": format_name,
                    "source": source.read_text(encoding="utf-8"), "mappings": mappings}
            report = call("xlsx_preview_prefill", args)
            assert report["credential_trust"] == trust, report
            changes = report["changes"]
            assert [row["cell"] for row in changes] == ["B4", "B5", "B6", "B7"], changes
            assert [row["target_label"] for row in changes] == [
                "申請者氏名", "受講した研修", "修了年月日", "研修記録の管理番号",
            ], changes
            assert [row["proposed_value"] for row in changes] == [
                None, "安全研修（サンプル）", "2026-04-01", number,
            ], changes
            assert [row["status"] for row in changes] == [
                "manual", "ready", "ready", "not_in_source" if number is None else "ready",
            ], changes
            assert all(change["current_value"] is None for change in changes), changes
            if format_name == "sd-jwt":
                completed = subprocess.run(
                    [str(build / "examples/prefill_preview"), str(workbook), str(source), str(mapping)],
                    encoding="utf-8", capture_output=True, check=True, timeout=10,
                )
                assert json.loads(completed.stdout) == report, completed.stdout
                denied = call("xlsx_apply_prefill", {
                    **args, "preview_id": report["preview_id"], "targets": ["training_name"],
                }, error=True)
                assert denied["error"]["message"] == "unknown tool xlsx_apply_prefill", denied
            duplicate = {**args, "mappings": [mappings[1], mappings[1]]}
            call("xlsx_preview_prefill", duplicate, error=True)
        cells = call("xlsx_get_cells", {"document_id": doc, "sheet": settings["sheet"], "range": "B4:B7"})
        assert cells["rows"] == [[None]] * 4, cells
    finally:
        server.stdin.close()

with TemporaryDirectory() as folder:
    populated = Path(folder) / "populated.xlsx"
    with ZipFile(workbook) as source_zip, ZipFile(populated, "w") as target_zip:
        for name in source_zip.namelist():
            data = source_zip.read(name)
            if name == "xl/worksheets/sheet1.xml":
                data = data.replace(b'<c r="B5" s="3"/>',
                                    b'<c r="B5" s="3" t="inlineStr"><is><t>Existing</t></is></c>')
                data = data.replace(b'<c r="B6" s="3"/>',
                                    b'<c r="B6" s="3"><f>1+1</f><v>2</v></c>')
            target_zip.writestr(name, data)
    completed = subprocess.run(
        [str(build / "examples/prefill_preview"), str(populated),
         str(fixtures / "training.sd-jwt"), str(mapping)],
        encoding="utf-8", capture_output=True, check=True, timeout=10,
    )
    occupied, formula = json.loads(completed.stdout)["changes"][1:3]
    assert occupied["status"] == "occupied" and occupied["current_value"] == "Existing", occupied
    assert formula["status"] == "blocked_target" and formula["current_value"] == 2, formula

assert workbook.read_bytes() == original
print("Prefill preview: four source formats, CLI/MCP parity, duplicate rejection, occupied and formula targets, and no edits")

with TemporaryDirectory() as folder:
    folder = Path(folder)
    binary = str(build / "examples/prefill_preview")
    presentation = fixtures / "training.sd-jwt"
    base = [binary, str(workbook), str(presentation), str(mapping)]
    review = folder / "review.json"
    reviewed = subprocess.run(base, encoding="utf-8", capture_output=True, check=True, timeout=10)
    review.write_text(reviewed.stdout, encoding="utf-8")
    output = folder / "filled.xlsx"
    decision = ["--apply", str(review), str(output)]
    result = subprocess.run([*base, *decision, "training_name"], capture_output=True, check=True, timeout=10)
    assert json.loads(result.stdout)["applied_targets"] == ["training_name"], result.stdout
    result = subprocess.run([binary, str(output), str(presentation), str(mapping)],
                            capture_output=True, check=True, timeout=10)
    filled = json.loads(result.stdout)["changes"]
    assert filled[1]["current_value"] == "安全研修（サンプル）" and filled[1]["status"] == "occupied", filled
    assert filled[2]["current_value"] is None and filled[3]["current_value"] is None, filled
    saved = output.read_bytes()
    duplicate = subprocess.run([*base, *decision, "training_name"], capture_output=True, timeout=10)
    assert duplicate.returncode != 0 and output.read_bytes() == saved, duplicate
    overwrite = subprocess.run([*base, "--apply", str(review), str(workbook), "training_name"],
                               capture_output=True, timeout=10)
    assert overwrite.returncode != 0 and workbook.read_bytes() == original, overwrite

    refused = folder / "refused.xlsx"
    blocked = subprocess.run([*base, "--apply", str(review), str(refused), "training_name", "record_number"],
                             capture_output=True, timeout=10)
    assert blocked.returncode != 0 and not refused.exists(), blocked
    source_changed = folder / "changed.sd-jwt"
    source_changed.write_bytes(presentation.read_bytes() + b"\n")
    stale = subprocess.run([binary, str(workbook), str(source_changed), str(mapping),
                            "--apply", str(review), str(refused), "training_name"], capture_output=True, timeout=10)
    assert stale.returncode != 0 and b"stale_preview" in stale.stderr and not refused.exists(), stale
    workbook_changed = folder / "changed.xlsx"
    workbook_changed.write_bytes(workbook.read_bytes())
    with ZipFile(workbook_changed, "a") as changed_zip:
        changed_zip.comment = b"edited after preview"
    stale = subprocess.run([binary, str(workbook_changed), str(presentation), str(mapping),
                            "--apply", str(review), str(refused), "training_name"], capture_output=True, timeout=10)
    assert stale.returncode != 0 and b"stale_preview" in stale.stderr and not refused.exists(), stale

    literal = folder / "literal.json"
    literal.write_text(json.dumps({"text": "=1+1", "too_large": 1234567890123456}), encoding="utf-8")
    literal_mapping = folder / "mapping.json"
    literal_mapping.write_text(json.dumps({"sheet": settings["sheet"], "mappings": [
        {"target": "training_name", "source_path": "/text"},
        {"target": "record_number", "source_path": "/too_large"},
    ]}), encoding="utf-8")
    base = [binary, str(workbook), str(literal), str(literal_mapping)]
    result = subprocess.run(base, encoding="utf-8", capture_output=True, check=True, timeout=10)
    assert json.loads(result.stdout)["changes"][1]["status"] == "unsupported_value", result
    review.write_text(result.stdout, encoding="utf-8")
    output = folder / "literal.xlsx"
    subprocess.run([*base, "--apply", str(review), str(output), "training_name"],
                   capture_output=True, check=True, timeout=10)
    result = subprocess.run([binary, str(output), str(literal), str(literal_mapping)],
                            capture_output=True, check=True, timeout=10)
    cell = json.loads(result.stdout)["changes"][0]
    assert cell["current_value"] == "=1+1" and cell["status"] == "occupied", cell

assert workbook.read_bytes() == original
print("Prefill apply: selected cells, stale review, refusal, and literal text")
