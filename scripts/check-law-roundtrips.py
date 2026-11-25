#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.

"""Check both XML round trips against pinned e-Gov revisions."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET


FIXTURES = Path(__file__).resolve().parents[1] / "crates/ibis-lawtext/tests/lawtext"
SCHEMA_URL = "https://laws.e-gov.go.jp/file/XMLSchemaForJapaneseLaw_v3.xsd"
SCHEMA_SHA256 = "f5e11ddb0d91c26232d6eb49a59001c3f5d418ef34fd2aef3b59223a6f633276"
DIFF_KEYS = ("xml_model_changes", "lawtext_roundtrip_changes", "markdown_roundtrip_changes")


def fetch(url):
    return subprocess.check_output(
        ["curl", "--fail", "--silent", "--show-error", "--location", "--retry", "3",
         "--connect-timeout", "10", "--max-time", "45", url]
    )


def verify(data, expected, label):
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected:
        raise ValueError(f"{label}: pinned input changed: {actual} (expected {expected})")
    return data


def signature(report):
    result = {}
    for key in DIFF_KEYS:
        changes = sorted((change["kind"], change["path"]) for change in report[key])
        encoded = json.dumps(changes, ensure_ascii=False, separators=(",", ":")).encode()
        result[key] = {"count": len(changes), "sha256": hashlib.sha256(encoded).hexdigest()}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("audit_binary", type=Path)
    parser.add_argument("report_directory", type=Path)
    args = parser.parse_args()
    directory = args.report_directory
    directory.mkdir(parents=True, exist_ok=True)

    schema = directory / "schema.xsd"
    schema.write_bytes(verify(fetch(SCHEMA_URL), SCHEMA_SHA256, "e-Gov XSD"))
    cases = json.loads((FIXTURES / "roundtrip-baseline.json").read_text())
    failures = []
    for name, case in cases.items():
        if "revision" in case:
            response = fetch(
                f"https://laws.e-gov.go.jp/api/2/law_data/{case['revision']}?response_format=xml"
            )
            law = ET.fromstring(response).find(".//Law")
            if law is None:
                raise ValueError(f"{name}: no Law element in the e-Gov response")
            data = ET.tostring(law, encoding="utf-8")
        else:
            data = (FIXTURES / case["fixture"]).read_bytes()
        input_path = directory / f"{name}.xml"
        input_path.write_bytes(verify(data, case["input_sha256"], name))
        audited = subprocess.run(
            [args.audit_binary.resolve(), input_path, "--xsd", schema],
            stdout=subprocess.PIPE,
            check=False,
        )
        report_path = directory / f"{name}.json"
        report_path.write_bytes(audited.stdout)
        if audited.returncode:
            failures.append(f"{name}: audit exited with {audited.returncode}; see {report_path}")
            continue
        report = json.loads(audited.stdout)
        actual = signature(report)
        counts = ", ".join(f"{key}={value['count']}" for key, value in actual.items())
        print(f"{name}: {counts}", flush=True)
        if actual != case["differences"]:
            failures.append(f"{name}: structural differences changed: {json.dumps(actual)}")
        if not all(report[key]["valid"] for key in ("output_xsd", "markdown_output_xsd")):
            failures.append(f"{name}: invalid XML; see {report_path}")
        if report["markdown_source_diagnostics"]:
            failures.append(f"{name}: Markdown source diagnostics; see {report_path}")
    if failures:
        raise SystemExit("\n".join(failures))


if __name__ == "__main__":
    main()
