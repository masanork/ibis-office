#!/usr/bin/env python3
"""Check PDF/A-2b export on existing synthetic Office documents."""
import argparse
import json
import os
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
from xml.etree import ElementTree

PDF_A_ID = "{http://www.aiim.org/pdfa/ns/id/}"
FIXTURES = (
    "test/corpus-report/docx/list-justify-cell-synthetic.docx",
    "test/web/fixtures/html-export.pptx",
    "test/fixtures/structured-preview/site-entry-form.xlsx",
)


def run(command):
    result = subprocess.run(command, capture_output=True, timeout=90)
    if result.returncode:
        raise RuntimeError(
            f"{command[0]} exited with {result.returncode}\n"
            + result.stderr.decode(errors="replace")
        )
    return result.stdout


def verify_metadata(pdf, archival):
    data = run(["pdfinfo", "-meta", pdf]).strip()
    values = {}
    if data:
        for node in ElementTree.fromstring(data).iter():
            if node.tag.startswith(PDF_A_ID):
                values[node.tag] = node.text
            values.update({key: value for key, value in node.attrib.items() if key.startswith(PDF_A_ID)})
    if archival:
        assert values.get(PDF_A_ID + "part") == "2", values
        assert values.get(PDF_A_ID + "conformance") == "B", values
    else:
        assert not values, values


def verify_identification(binary, pdf, archival):
    report = json.loads(run([binary, "--export", "pdfa-info", pdf]))
    assert report == {
        "declared_profile": "PDF/A-2b" if archival else None,
        "part": "2" if archival else None,
        "conformance": "B" if archival else None,
        "revision": None,
        "validation": "not_run",
    }, report


def verify_conformance(binary, validator, pdf, compliant, profile="2b"):
    result = subprocess.run(
        [binary, "--export", "pdfa-validate", "--pdfa-profile", profile,
         "--verapdf", validator, pdf],
        capture_output=True, timeout=100,
    )
    assert result.returncode == (0 if compliant else 3), result
    report = json.loads(result.stdout)
    assert report["profile"] == f"PDF/A-{profile}", report
    assert report["validator"] == "veraPDF", report
    assert report["validation"] == ("compliant" if compliant else "non_compliant"), report
    assert bool(report["failed_rules"]) != compliant, report
    if not compliant:
        assert all(rule["clause"] and rule["description"] and rule["failed_checks"] > 0
                   for rule in report["failed_rules"]), report


def verify_validator_errors(binary, pdf):
    for options, expected_code in [
        ([], 2),
        (["--pdfa-profile", "0"], 2),
        (["--pdfa-profile", "2b", "--verapdf", str(pdf.parent / "missing-validator")], 1),
    ]:
        result = subprocess.run(
            [binary, "--export", "pdfa-validate", *options, pdf],
            capture_output=True, timeout=15,
        )
        assert result.returncode == expected_code and not result.stdout, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--verapdf", default=os.environ.get("VERAPDF"),
                        help="optional path to the independent veraPDF CLI")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    root = Path(__file__).resolve().parent.parent
    with TemporaryDirectory() as directory:
        directory = Path(directory)
        for fixture in FIXTURES:
            source = root / fixture
            run([binary, "--export", "pdfa-2b", "--out-dir", directory, source])
            pdf = directory / (source.stem + ".pdf")
            verify_metadata(pdf, archival=True)
            verify_identification(binary, pdf, archival=True)
            assert run(["pdftotext", pdf, "-"]).strip(), fixture
            if args.verapdf:
                verify_conformance(binary, args.verapdf, pdf, compliant=True)
                if fixture == FIXTURES[0]:
                    verify_conformance(binary, args.verapdf, pdf, compliant=False, profile="1b")
            print(f"{source.suffix[1:]}: PDF/A-2b metadata and text OK"
                  + ("; veraPDF compliant" if args.verapdf else ""))
        source = root / FIXTURES[-1]
        run([binary, "--export", "pdf", "--out-dir", directory, source])
        pdf = directory / (source.stem + ".pdf")
        verify_metadata(pdf, archival=False)
        verify_identification(binary, pdf, archival=False)
        verify_validator_errors(binary, pdf)
        if args.verapdf:
            verify_conformance(binary, args.verapdf, pdf, compliant=False)
            print("ordinary PDF: veraPDF reported non-compliance and failed rules")


if __name__ == "__main__":
    main()
