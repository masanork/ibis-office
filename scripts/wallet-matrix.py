#!/usr/bin/env python3
"""Run synthetic Wallet regression cases from an XLSX sheet."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

HEADERS = ["case", "format", "scenario", "expected_failures"]
RUNNER = Path(__file__).with_name("wallet-run.py")


def read_rows(workbook, sheet, build):
    with subprocess.Popen(
        [str(build / "ibisOffice-mcp"), "--read-only", "--allow-dir", str(workbook.parent),
         "--max-file-size", "20"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        text=True, encoding="utf-8",
    ) as server:
        def call(name, args):
            server.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                           "params": {"name": name, "arguments": args}}) + "\n")
            server.stdin.flush()
            reply = json.loads(server.stdout.readline())["result"]
            if reply["isError"]:
                raise ValueError(f"{name}: {reply['structuredContent']['error']['message']}")
            return reply["structuredContent"]

        try:
            document = call("app_open_document", {"path": str(workbook)})["document_id"]
            address = {"document_id": document, "sheet": sheet}
            used = call("xlsx_get_used_range", address)["used_range"] or ""
            match = re.fullmatch(r"A1:D([1-9][0-9]*)", used)
            if not match or not 2 <= int(match[1]) <= 101:
                raise ValueError("case sheet must fill columns A:D from row 1 and contain 1–100 cases")
            # Read formula text to keep the manifest literal and reject formula-driven cases.
            return call("xlsx_get_cells", {**address, "range": used, "kind": "formula", "limit": 101})["rows"]
        finally:
            server.stdin.close()
            try:
                server.wait(timeout=2)
            except subprocess.TimeoutExpired:
                server.kill()


def cases_from_rows(rows):
    if rows[0] != HEADERS:
        raise ValueError("row 1 must contain case, format, scenario, expected_failures")
    cases = []
    names = set()
    for number, (name, format_name, scenario, failures) in enumerate(rows[1:], start=2):
        if all(value is None for value in (name, format_name, scenario, failures)):
            continue
        if not isinstance(name, str) or not name.strip() or name.startswith("=") or name in names:
            raise ValueError(f"row {number}: case name must be nonempty, literal and unique")
        if format_name not in ("sd-jwt", "mdoc") or scenario not in ("normal", "extra", "wrong-nonce"):
            raise ValueError(f"row {number}: invalid fixture format or scenario")
        if format_name == "mdoc" and scenario == "wrong-nonce":
            raise ValueError(f"row {number}: wrong-nonce is only available for SD-JWT")
        if failures is not None and not isinstance(failures, str):
            raise ValueError(f"row {number}: expected_failures must be text or empty")
        checks = [part.strip() for part in (failures or "").split(",")] if failures else []
        if any(not re.fullmatch(r"[a-z][a-z_]*", check) for check in checks) or len(checks) != len(set(checks)):
            raise ValueError(f"row {number}: invalid expected failure names")
        names.add(name)
        cases.append((name, format_name, scenario, sorted(checks)))
    if not cases:
        raise ValueError("case sheet contains no test cases")
    return cases


def run_cases(cases, build):
    reports = []
    for name, format_name, scenario, expected in cases:
        result = subprocess.run(
            [sys.executable, str(RUNNER), format_name, "--scenario", scenario, "--build-dir", str(build)],
            text=True, encoding="utf-8", capture_output=True, timeout=45,
        )
        if result.returncode not in (0, 1):
            raise ValueError(f"Wallet execution failed for case {name!r}")
        comparison = json.loads(result.stdout)
        actual = sorted(comparison["failed_checks"])
        reports.append({"case": name, "format": format_name, "scenario": scenario,
                        "expected_failures": expected, "actual_failures": actual,
                        "passed": expected == actual, "report": comparison["report"]})
    return reports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workbook", type=Path)
    parser.add_argument("--sheet", default="Wallet Cases")
    parser.add_argument("--build-dir", type=Path, default=Path("target/debug"))
    args = parser.parse_args()
    try:
        workbook = args.workbook.resolve(strict=True)
        if workbook.suffix.lower() != ".xlsx" or not workbook.is_file():
            raise ValueError("case manifest must be an XLSX file")
        build = args.build_dir.resolve()
        reports = run_cases(cases_from_rows(read_rows(workbook, args.sheet, build)), build)
        passed = all(report["passed"] for report in reports)
        print(json.dumps({"passed": passed, "cases": reports}))
        return int(not passed)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"wallet matrix: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
