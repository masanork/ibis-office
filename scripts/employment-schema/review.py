#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Inspect a mapping candidate and explicitly record a human approval."""

import argparse
import hashlib
import json
from pathlib import Path
import re

GROUPS = {
    "common": "事業所・本人・証明日・雇用形態",
    "fixed": "固定勤務",
    "variable": "変則勤務",
    "excluded": "保護者記載欄の除外",
}
BASELINE = json.loads(Path(__file__).with_name("mapping.json").read_text(encoding="utf-8"))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def cell_ref(value):
    match = re.fullmatch(r"([A-Z]{1,2})([1-9][0-9]?)", value) if isinstance(value, str) else None
    require(match is not None, f"Invalid cell reference: {value!r}")
    letters, row = match.groups()
    column = 0
    for letter in letters:
        column = column * 26 + ord(letter) - ord("A") + 1
    require(2 <= column <= 37 and int(row) <= 50, f"Outside the company section: {value}")


def check_structure(value, reference, seen, name="mapping"):
    """Keep the pilot's supported fields and transformations while allowing cell corrections."""
    if isinstance(reference, dict):
        require(isinstance(value, dict) and value.keys() == reference.keys(), f"{name}: unexpected fields")
        for key, sample in reference.items():
            check_structure(value[key], sample, seen, f"{name}.{key}")
    elif isinstance(reference, list):
        require(isinstance(value, list) and len(value) == len(reference), f"{name}: unexpected length")
        for item, sample in zip(value, reference):
            check_structure(item, sample, seen, name)
    elif isinstance(reference, str) and re.fullmatch(r"[A-Z]{1,2}[1-9][0-9]?", reference):
        cell_ref(value)
        require(value not in seen, f"{name}: cell {value} is already assigned to {seen.get(value)}")
        seen[value] = name
    else:
        require(type(value) is type(reference) and value == reference, f"{name}: unsupported change")


def mapping_digest(mapping):
    content = json.dumps(mapping, ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha256(content.encode("utf-8")).hexdigest()


def reviewed_mapping(draft, reviewer):
    require(isinstance(draft, dict) and isinstance(draft.get("checks"), dict), "Missing review checks")
    require(isinstance(reviewer, str) and reviewer.strip(), "A reviewer name is required")
    require(draft["checks"].keys() == GROUPS.keys(), "All four review groups are required")
    for key, label in GROUPS.items():
        check = draft["checks"][key]
        require(isinstance(check, dict) and check.get("decision") == "accepted", f"{label}: approval pending")
        for field in ("evidence", "review_note"):
            require(isinstance(check.get(field), str) and check[field].strip(), f"{label}: {field} is required")
    mapping = draft.get("mapping")
    check_structure(mapping, BASELINE, {})
    return {
        "mapping": mapping,
        "approval": {"reviewer": reviewer.strip(), "sha256": mapping_digest(mapping), "checks": draft["checks"]},
    }


def load_approved(path):
    document = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(document, dict) and isinstance(document.get("approval"), dict), "An approved mapping is required")
    approval = document["approval"]
    mapping = document.get("mapping")
    checked = reviewed_mapping({"mapping": mapping, "checks": approval.get("checks")}, approval.get("reviewer"))
    require(approval.get("sha256") == checked["approval"]["sha256"], "Mapping changed after approval")
    return mapping


def review_summary(draft):
    require(isinstance(draft, dict) and isinstance(draft.get("mapping"), dict), "Missing candidate mapping")
    mapping = draft["mapping"]
    schedules = mapping.get("schedules")
    checks = draft.get("checks")
    require(isinstance(schedules, dict) and isinstance(checks, dict), "Missing schedules or review checks")
    groups = {"common": mapping.get("common"), **schedules, "excluded": mapping.get("excluded")}
    lines = [f'{mapping.get("template", "")} / {mapping.get("sheet", "")}']
    for key, label in GROUPS.items():
        check = checks.get(key, {})
        require(isinstance(check, dict), f"{label}: invalid review check")
        lines.extend((
            "",
            f"## {label} [{check.get('decision', 'pending')}]",
            f"根拠  {check.get('evidence', '')}",
            f"要確認  {check.get('question', '')}",
            f"担当者メモ  {check.get('review_note', '')}",
            json.dumps(groups.get(key), ensure_ascii=False, indent=2),
        ))
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("show", "approve"))
    parser.add_argument("draft", type=Path)
    parser.add_argument("--reviewer")
    args = parser.parse_args()
    try:
        draft = json.loads(args.draft.read_text(encoding="utf-8"))
        if args.command == "show":
            print(review_summary(draft))
        else:
            print(json.dumps(reviewed_mapping(draft, args.reviewer), ensure_ascii=False, indent=2))
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
