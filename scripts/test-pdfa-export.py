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


def verify_conformance(validator, pdf):
    xml = run([validator, "--format", "xml", "--flavour", "2b", pdf])
    reports = ElementTree.fromstring(xml).findall(".//{*}validationReport")
    assert len(reports) == 1 and reports[0].get("isCompliant") == "true", xml.decode(errors="replace")


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
                verify_conformance(args.verapdf, pdf)
            print(f"{source.suffix[1:]}: PDF/A-2b metadata and text OK"
                  + ("; veraPDF compliant" if args.verapdf else ""))
        source = root / FIXTURES[-1]
        run([binary, "--export", "pdf", "--out-dir", directory, source])
        pdf = directory / (source.stem + ".pdf")
        verify_metadata(pdf, archival=False)
        verify_identification(binary, pdf, archival=False)


if __name__ == "__main__":
    main()
