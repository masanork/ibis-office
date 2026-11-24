#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Run the offline synthetic employment comparison against both generated queries."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

from pilot import MAPPING, extract
from presentation import dcql
from review import reviewed_mapping

HERE = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path, help="path to the employment_presentation example")
    args = parser.parse_args()
    record = extract({
        "Z4": "架空事業所", "I19": "架空従業員", "AA3": 2026, "AF3": 11, "AI3": 23,
        "L23": "☑", "M31": "☑", "P32": "☑", "S31": 96,
        "W31": 30, "AC31": 480, "S32": 4,
    })
    draft = json.loads((HERE / "candidate.example.json").read_text(encoding="utf-8"))
    draft["mapping"] = MAPPING
    for check in draft["checks"].values():
        check.update(decision="accepted", review_note="Synthetic regression fixture")
    with tempfile.TemporaryDirectory() as directory:
        workspace = Path(directory)
        approved = reviewed_mapping(draft, "Synthetic regression")
        (workspace / "approved.json").write_text(json.dumps(approved), encoding="utf-8")
        shutil.copyfile(HERE / "employment.example.md", workspace / "employment.md")
        request = workspace / "childcare-request.md"
        shutil.copyfile(HERE / "childcare-request.example.md", request)
        inputs = {"record": record, "sd-jwt": dcql(request), "mdoc": dcql(request, "mso_mdoc")}
        subprocess.run([args.fixture.resolve()], input=json.dumps(inputs), text=True, check=True)


if __name__ == "__main__":
    main()
