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
        result = json.loads(server.stdout.readline())["result"]
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
