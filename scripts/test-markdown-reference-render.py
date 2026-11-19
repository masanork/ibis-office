#!/usr/bin/env python3
"""Render the Japanese Markdown reference with a desktop build and check its printed text."""

import argparse
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory


REFERENCE = Path(__file__).resolve().parents[1] / "docs" / "markdown-reference-ja.md"
SECTION_HEADINGS = (
    "Docs Markdown Reference Manual",
    "Office Open Markdown の往復確認",
    "表のセル結合",
    "Markdown 構文一覧",
    "Mermaid",
    "法令標準XML と Lawtext",
    "AI 向け JSON 書き出し",
    "ステージング文書としての更新",
)
LAST_SENTENCE = "この文書は機能を組み合わせたときの表示と操作を確認するために使います。"
PDFKIT_EXTRACT = r"""
import Foundation
import PDFKit

guard let document = PDFDocument(url: URL(fileURLWithPath: CommandLine.arguments[1])) else {
    fatalError("Could not open rendered PDF")
}
for index in 0..<document.pageCount {
    if index > 0 { print("\u{000C}", terminator: "") }
    print(document.page(at: index)?.string ?? "")
}
"""


def pdf_pages(filename):
    if sys.platform == "darwin":
        text = subprocess.check_output(
            ["swift", "-", str(filename)], input=PDFKIT_EXTRACT, text=True, encoding="utf-8"
        )
    else:
        text = subprocess.check_output(
            ["pdftotext", "-enc", "UTF-8", str(filename), "-"], text=True, encoding="utf-8"
        )
    return ["".join(page.split()) for page in text.rstrip("\n\r\f").split("\f")]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()

    with TemporaryDirectory(prefix="ibis-reference-render-") as directory:
        output = Path(directory)
        subprocess.run(
            [str(args.binary.resolve()), "--export", "pdf", "--out-dir", directory, str(REFERENCE)],
            check=True,
        )
        pages = pdf_pages(output / REFERENCE.with_suffix(".pdf").name)

    text = "".join(pages)
    cursor = 0
    for heading in SECTION_HEADINGS:
        anchor = "".join(heading.split())
        position = text.find(anchor, cursor)
        if position < 0:
            raise SystemExit(f"Rendered reference is missing or out of order at: {heading}")
        cursor = position + len(anchor)
    if LAST_SENTENCE not in pages[-1] or LAST_SENTENCE not in text[cursor:]:
        raise SystemExit("Rendered reference is missing the closing sentence on its final page")
    print(f"Japanese Markdown reference rendered through its closing sentence ({len(pages)} pages)")


if __name__ == "__main__":
    main()
