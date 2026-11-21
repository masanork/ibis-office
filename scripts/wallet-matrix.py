#!/usr/bin/env python3
"""Run Wallet regression cases from an XLSX sheet."""
import argparse
from contextlib import contextmanager, nullcontext
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from tempfile import TemporaryDirectory

RUNNER = Path(__file__).with_name("wallet-run.py")


@contextmanager
def mcp_session(workbook, build, report_dir=None):
    command = [str(build / "ibisOffice-mcp"), "--allow-dir", str(workbook.parent),
               "--max-file-size", "20"]
    if report_dir is None:
        command.append("--read-only")
    else:
        command.extend(["--allow-dir", str(report_dir)])
    with subprocess.Popen(
        command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
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
            yield call
        finally:
            server.stdin.close()
            try:
                server.wait(timeout=2)
            except subprocess.TimeoutExpired:
                server.kill()


def read_rows(call, address):
    used = call("xlsx_get_used_range", address)["used_range"] or ""
    match = re.fullmatch(r"A1:D([1-9][0-9]*)", used)
    if not match or not 2 <= int(match[1]) <= 101:
        raise ValueError("case sheet must fill columns A:D from row 1 and contain 1–100 cases")
    # Read formula text to keep the manifest literal and reject formula-driven cases.
    return call("xlsx_get_cells", {**address, "range": used, "kind": "formula", "limit": 101})["rows"]


def write_report(call, address, reports, stage, destination):
    values = [[None, None] for _ in range(reports[-1]["row"])]
    values[0] = ["actual_failures", "result"]
    for report in reports:
        values[report["row"] - 1] = [
            ", ".join(report["actual_failures"]) or None,
            "PASS" if report["passed"] else "FAIL",
        ]
    call("xlsx_set_cells", {**address, "range": f"E1:F{len(values)}", "values": values})
    temporary = stage / "report.xlsx"
    result = call("app_save_document", {"document_id": address["document_id"], "path": str(temporary)})
    # The staged file is on the destination filesystem. Publish it atomically
    # and fail if another process created the destination while we ran cases.
    os.link(temporary, destination)
    return {"path": str(destination), "warnings": result["warnings"]}


def cases_from_rows(rows, verifier_dir):
    source = "verifier" if verifier_dir is not None else "scenario"
    headers = ["case", "format", source, "expected_failures"]
    if rows[0] != headers:
        raise ValueError("row 1 must contain " + ", ".join(headers))
    cases = []
    names = set()
    for number, (name, format_name, selection, failures) in enumerate(rows[1:], start=2):
        if all(value is None for value in (name, format_name, selection, failures)):
            continue
        if not isinstance(name, str) or not name.strip() or name.startswith("=") or name in names:
            raise ValueError(f"row {number}: case name must be nonempty, literal and unique")
        if format_name not in ("sd-jwt", "mdoc"):
            raise ValueError(f"row {number}: invalid format")
        if verifier_dir is None:
            if selection not in ("normal", "extra", "wrong-nonce"):
                raise ValueError(f"row {number}: invalid fixture scenario")
            if format_name == "mdoc" and selection == "wrong-nonce":
                raise ValueError(f"row {number}: wrong-nonce is only available for SD-JWT")
            options = ["--scenario", selection]
        else:
            if not isinstance(selection, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]*\.json", selection):
                raise ValueError(f"row {number}: verifier must be a JSON filename")
            path = (verifier_dir / selection).resolve()
            if path.parent != verifier_dir or not path.is_file():
                raise ValueError(f"row {number}: verifier file must exist in --verifier-dir")
            options = ["--request-settings", str(path)]
        if failures is not None and not isinstance(failures, str):
            raise ValueError(f"row {number}: expected_failures must be text or empty")
        checks = [part.strip() for part in (failures or "").split(",")] if failures else []
        if any(not re.fullmatch(r"[a-z][a-z_]*", check) for check in checks) or len(checks) != len(set(checks)):
            raise ValueError(f"row {number}: invalid expected failure names")
        names.add(name)
        cases.append((number, name, format_name, source, selection, sorted(checks), options))
    if not cases:
        raise ValueError("case sheet contains no test cases")
    return cases


def run_cases(cases, build, wallet_command):
    reports = []
    for number, name, format_name, source, selection, expected, options in cases:
        command = [sys.executable, str(RUNNER), format_name, "--build-dir", str(build), *options]
        if wallet_command:
            command.extend(["--wallet-command", *wallet_command])
        result = subprocess.run(
            command,
            text=True, encoding="utf-8", capture_output=True, timeout=45,
        )
        if result.returncode not in (0, 1):
            raise ValueError(f"Wallet execution failed for case {name!r}")
        comparison = json.loads(result.stdout)
        actual = sorted(comparison["failed_checks"])
        reports.append({"row": number, "case": name, "format": format_name, source: selection,
                        "expected_failures": expected, "actual_failures": actual,
                        "passed": expected == actual, "report": comparison["report"]})
    return reports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workbook", type=Path)
    parser.add_argument("--sheet", default="Wallet Cases")
    parser.add_argument("--build-dir", type=Path, default=Path("target/debug"))
    parser.add_argument("--report-xlsx", type=Path, help="save results in a new workbook")
    parser.add_argument("--verifier-dir", type=Path, help="directory of row-selected verifier JSON files")
    parser.add_argument("--wallet-command", nargs=argparse.REMAINDER, metavar="ARG",
                        help="external command shared by all rows (must come last)")
    args = parser.parse_args()
    if args.wallet_command == [] or bool(args.wallet_command) != bool(args.verifier_dir):
        parser.error("--verifier-dir and a nonempty --wallet-command must be supplied together")
    try:
        workbook = args.workbook.resolve(strict=True)
        if workbook.suffix.lower() != ".xlsx" or not workbook.is_file():
            raise ValueError("case manifest must be an XLSX file")
        build = args.build_dir.resolve()
        verifier_dir = args.verifier_dir.resolve(strict=True) if args.verifier_dir else None
        if verifier_dir is not None and not verifier_dir.is_dir():
            raise ValueError("--verifier-dir must be a directory")
        destination = args.report_xlsx
        if destination is not None:
            destination = destination.parent.resolve(strict=True) / destination.name
            if destination.suffix.lower() != ".xlsx" or destination.exists() or destination.is_symlink():
                raise ValueError("--report-xlsx must name a new XLSX file")
        staging = TemporaryDirectory(prefix=".wallet-report-", dir=destination.parent) if destination else nullcontext()
        with staging as folder:
            stage = Path(folder) if folder is not None else None
            with mcp_session(workbook, build, stage) as call:
                document = call("app_open_document", {"path": str(workbook)})["document_id"]
                address = {"document_id": document, "sheet": args.sheet}
                cases = cases_from_rows(read_rows(call, address), verifier_dir)
                reports = run_cases(cases, build, args.wallet_command)
                passed = all(report["passed"] for report in reports)
                result = {"passed": passed, "cases": reports}
                if destination is not None:
                    result["report_xlsx"] = write_report(call, address, reports, stage, destination)
        print(json.dumps(result))
        return int(not passed)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"wallet matrix: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
