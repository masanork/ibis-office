#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Test DOCX AI edit preview, save, and receipts with a built ibisOffice CLI."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def run(binary, *arguments, error=None):
    result = subprocess.run(
        [str(binary), *map(str, arguments)],
        capture_output=True,
        encoding="utf-8",
        timeout=30,
    )
    if error is None:
        assert result.returncode == 0, result.stderr
    else:
        assert result.returncode == 1, result
        assert error in result.stderr, result.stderr
        assert not result.stdout, result.stdout
    return result.stdout


def byte_range(text, quote):
    encoded = quote.encode("utf-8")
    start = text.encode("utf-8").index(encoded)
    return [start, start + len(encoded)]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def test_workflow(binary, directory):
    source = directory / "original.docx"
    markdown = directory / "original.md"
    markdown.write_text(
        "出張先を確認する。\n\n| 支店 |\n| --- |\n| **東京**と*大阪* |\n",
        encoding="utf-8",
    )
    run(binary, "--export", "docx", "--out-dir", directory, markdown)
    run(binary, "--export", "ai-json", "--out-dir", directory, source)
    export = json.loads((directory / "original.ai.json").read_text(encoding="utf-8"))
    source_hash = digest(source)
    assert export["sourceSha256"] == source_hash

    original_cell = "東京と大阪"
    changes = [
        {
            "markdownBytes": byte_range(export["markdown"], "出張先を確認する。"),
            "quote": "出張先を確認する。",
            "before": "出張先を確認する。",
            "replacement": "出張先を確定する。",
        }
    ]
    for quote, replacement in [("東京", "横浜市"), ("大阪", "札幌")]:
        changes.append({
            "target": "tableCell",
            "markdownBytes": byte_range(export["markdown"], quote),
            "paragraphBytes": byte_range(original_cell, quote),
            "quote": quote,
            "before": original_cell,
            "replacement": replacement,
        })
    proposal = {"version": 1, "sourceSha256": source_hash, "edits": changes}
    query = directory / "batch.json"

    def write_proposal():
        query.write_text(json.dumps(proposal, ensure_ascii=False), encoding="utf-8")

    def apply(output, error=None):
        return run(
            binary, "--export", "ai-apply", "--require-preview", "--ai-receipt",
            "--ai-query", query, "--out-file", output, source, error=error,
        )

    write_proposal()
    preview = json.loads(run(binary, "--export", "ai-preview", "--ai-query", query, source))
    assert [target["after"] for target in preview["targets"]] == ["出張先を確定する。", "横浜市と札幌"]
    assert preview["targets"][1]["editIndices"] == [2, 3]

    rejected = directory / "rejected.docx"
    apply(rejected, error="needs previewSha256")
    assert not rejected.exists()
    proposal["previewSha256"] = preview["previewSha256"]
    changes[2]["replacement"] = "仙台"
    write_proposal()
    apply(rejected, error="differs from the checked preview")
    assert not rejected.exists()

    changes[2]["replacement"] = "札幌"
    write_proposal()
    output = directory / "reviewed.docx"
    receipt = json.loads(apply(output))
    output_hash = digest(output)
    assert Path(receipt["outputPath"]) == output
    assert receipt["sourceSha256"] == source_hash
    assert receipt["outputSha256"] == output_hash
    assert receipt["previewSha256"] == preview["previewSha256"]
    assert receipt["previewVerified"] is True and receipt["editCount"] == 3

    # A second write must fail without a receipt, even with the same proposal.
    apply(output, error=str(output))
    assert digest(output) == output_hash
    assert digest(source) == source_hash
    run(binary, "--export", "md", "--out-dir", directory, output)
    reviewed = (directory / "reviewed.md").read_text(encoding="utf-8")
    assert "出張先を確定する。" in reviewed
    assert "| **横浜市**と*札幌* |" in reviewed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="ibis-ai-edit-") as directory:
        test_workflow(args.binary.resolve(strict=True), Path(directory).resolve())
    print("DOCX AI edit CLI workflow passed")


if __name__ == "__main__":
    main()
