#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Review the approved employment export against a saved XLSX in ibisOffice."""

import argparse
import base64
from html import escape
import json
from pathlib import Path
import subprocess

from okf import load_concept
from pilot import extract, field_operation, profile

LABELS = {
    "employer_name": "証明事業所名",
    "employee_name": "本人氏名",
    "certification_date": "証明日",
    "employment_type": "雇用形態",
    "work_schedule": "勤務区分",
    "work_schedule.total": "就労時間の合計",
    "work_schedule.days": "就労日数",
    "work_schedule.days_per_month": "月間の就労日数",
    "work_schedule.days_per_week": "週間の就労日数",
    "unit": "単位", "hours": "時間", "minutes": "分", "break_minutes": "うち休憩分", "count": "日数",
}
VALUE_LABELS = {"employee": "正社員", "part_time": "パート・アルバイト",
                "fixed": "固定", "variable": "変則", "month": "月間", "week": "週間"}


def read_workbook(command, workbook, sheet):
    options = [] if command.name.lower() == "ibisoffice-mcp.exe" else ["--mcp"]
    with subprocess.Popen(
        [command, *options, "--read-only", "--allow-dir", workbook.parent],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding="utf-8",
    ) as server:
        number = 0

        def send(method, params, notify=False):
            nonlocal number
            number += 1
            message = {"jsonrpc": "2.0", "method": method, "params": params}
            if not notify:
                message["id"] = number
            server.stdin.write(json.dumps(message) + "\n")
            server.stdin.flush()
            if notify:
                return None
            line = server.stdout.readline()
            if not line:
                raise ValueError("ibisOffice closed its MCP connection")
            reply = json.loads(line)
            if reply.get("id") != number or "result" not in reply:
                raise ValueError(f"ibisOffice MCP request failed: {reply.get('error', method)}")
            return reply["result"]

        def call(name, **arguments):
            result = send("tools/call", {"name": name, "arguments": arguments})
            if result.get("isError") or "structuredContent" not in result:
                message = next((item["text"] for item in result.get("content", []) if item.get("type") == "text"), name)
                raise ValueError(message)
            return result["structuredContent"]

        try:
            send("initialize", {
                "protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "employment-workbook-review", "version": "1"},
            })
            send("notifications/initialized", {}, notify=True)
            handle = call("app_open_document", path=str(workbook))["document_id"]
            values = call("xlsx_get_cells", document_id=handle, sheet=sheet, range="B1:AK50")
        finally:
            server.terminate()
    if values["first_row"] != 1 or values["first_column"] != "B" or len(values["rows"]) != 50:
        raise ValueError("ibisOffice did not return the complete company section")
    columns = [chr(65 + index) for index in range(1, 26)] + [f"A{char}" for char in "ABCDEFGHIJK"]
    if any(len(row) != len(columns) for row in values["rows"]):
        raise ValueError("ibisOffice did not return the complete company section")
    return {f"{column}{row_number}": value
            for row_number, row in enumerate(values["rows"], 1)
            for column, value in zip(columns, row)}


def source_rows(rule, values, cells, parent=""):
    for key, child in rule.items():
        path = f"{parent}.{key}" if parent else key
        operation = field_operation(child)
        if operation is None:
            yield from source_rows(child, values[key], cells, path)
            continue
        if operation == "constant":
            source = "勤務時間欄の様式定義"
        else:
            if operation == "choice":
                refs = [child["choice"][values[key]]]
            elif operation == "date":
                refs = child["date"]
            else:
                refs = [child[operation]]
            source = "・".join(f"{ref} = {cells[ref]}" for ref in refs)
        group = LABELS.get(parent) if key != "kind" else None
        label = LABELS.get(path, LABELS.get(key, "勤務区分" if key == "kind" else key))
        if group:
            label = f"{group}／{label}"
        value = values[key]
        shown = VALUE_LABELS.get(value, value)
        yield label, shown, source


def render_review(workbook, mapping, cells):
    # Spot-check the published template before applying its positional mapping.
    for ref, expected in {"B1": "就労証明書", "B23": 5, "B25": 6,
                        "I31": "合計時間", "I32": "就労日数"}.items():
        if cells.get(ref) != expected:
            raise ValueError(f"{ref}: the worksheet does not match the pilot template")
    record = extract(cells, mapping)
    kind = record["work_schedule"]["kind"]
    fields = profile(kind, mapping["schedules"][kind], mapping)
    rows = "\n".join(
        f"<tr><th scope='row'>{escape(label)}</th><td>{escape(str(value))}</td><td>{escape(source)}</td></tr>"
        for label, value, source in source_rows(fields, record, cells)
    )
    payload = json.dumps(record, ensure_ascii=False, indent=2) + "\n"
    download = base64.b64encode(payload.encode("utf-8")).decode("ascii")
    name, sheet = escape(workbook.name), escape(mapping["sheet"])
    return f"""<!doctype html>
<html lang="ja">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">
<title>就労証明書の受け渡し確認</title>
<style>
  body {{ margin: 0; background: #f3f5f4; color: #172821; font: 1rem/1.55 system-ui, sans-serif; }}
  main {{ max-width: 68rem; margin: auto; padding: clamp(1rem, 4vw, 3rem); }}
  h1 {{ font-size: clamp(1.4rem, 4vw, 2rem); margin: 0; }}
  table {{ width: 100%; background: white; border-collapse: collapse; text-align: left; }}
  td, th {{ padding: .65rem .8rem; border-bottom: 1px solid #d7e0dc; overflow-wrap: anywhere; }}
  thead {{ background: #e3ede8; }}
  a {{ display: inline-block; padding: .6rem 1rem; border-radius: .35rem; color: white; background: #154d39; }}
  a:focus-visible {{ outline: 3px solid #236849; outline-offset: 3px; }}
</style>
<main>
  <h1>就労証明書の受け渡し確認</h1>
  <p>{name} ／ {sheet}</p>
  <table><thead><tr><th scope="col">項目</th><th scope="col">渡す値</th><th scope="col">元のセルと入力値</th></tr></thead>
  <tbody>{rows}</tbody></table>
  <p>内容を変更する場合はXLSXを修正して保存し、この確認画面を作り直してください。</p>
  <a download="employment-claims.json" href="data:application/json;base64,{download}">就労データをJSONで保存</a>
</main>
</html>
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workbook", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--okf", type=Path, required=True)
    parser.add_argument("--ibis-office", type=Path, default=Path("ibisOffice"))
    args = parser.parse_args()
    try:
        workbook = args.workbook.resolve(strict=True)
        if workbook.suffix.lower() != ".xlsx":
            raise ValueError("Expected a saved XLSX workbook")
        mapping = load_concept(args.okf)
        cells = read_workbook(args.ibis_office, workbook, mapping["sheet"])
        page = render_review(workbook, mapping, cells)
        with args.output.open("x", encoding="utf-8") as output:
            output.write(page)
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
