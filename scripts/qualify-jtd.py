#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Compare the real Rust importer with independent logical-text decoding.

Build first: cargo build -p ibis-pptx --example jtd_import
Input files stay local. Only counts and filenames are printed; no text.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

spec = importlib.util.spec_from_file_location("audit_jtd", Path(__file__).with_name("audit-jtd.py"))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def qualify(binary, samples):
    results = []
    with tempfile.TemporaryDirectory(prefix="ibis-jtd-qualification-") as temporary:
        for index, path in enumerate(samples):
            output = Path(temporary) / f"{index}.docx"
            imported = subprocess.run([str(binary), str(path), str(output)], capture_output=True, text=True)
            if imported.returncode:
                results.append({"file": path.name, "ok": False, "error": imported.stderr.strip()})
                continue
            stats = json.loads(imported.stdout)
            source = audit.audit(path, docx=output)
            comparison = source["docx"]["base_text_comparison"]
            ruby = source["docx"]["ruby_text_comparison"]
            headers_match = stats["ruled_headers"] == source["grammar"]["ruled_headers"]
            # This adapter intentionally represents each source display row
            # as one Word row; it does not synthesize logical merged rows.
            rows_match = source["docx"]["rows"] == source["grammar"]["ruled_headers"]
            results.append({"file": path.name, "ok": comparison["same_sequence"] and ruby["same_sequence"] and headers_match and rows_match,
                "comparison": comparison, "ruby_comparison": ruby, "ruled_headers_match": headers_match,
                "display_rows_match": rows_match,
                "docx_default_fonts": source['docx']['default_fonts'],
                "docx_default_grid_pitch_pt": source['docx']['default_grid_pitch_pt'],
                "rust": stats, "docx_rows": source["docx"]["rows"]})
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("samples", type=Path, help="a JTD file or directory of .jtd files")
    parser.add_argument("--binary", type=Path, default=Path("target/debug/examples/jtd_import"))
    parser.add_argument("--json-out", type=Path)
    args = parser.parse_args()
    samples = sorted(args.samples.glob("*.jtd")) if args.samples.is_dir() else [args.samples]
    if not samples:
        raise SystemExit("no .jtd inputs")
    results = qualify(args.binary.resolve(), samples)
    if args.json_out:
        args.json_out.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    for result in results:
        print(json.dumps(result, ensure_ascii=False))
    if not all(result["ok"] for result in results):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
