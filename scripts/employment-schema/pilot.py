#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Generate a JSON Schema or extract employment claims from an A1 cell/value map."""

import argparse
from datetime import date
import json
from pathlib import Path

MAPPING = json.loads(Path(__file__).with_name("mapping.json").read_text(encoding="utf-8"))
OPERATIONS = ("text", "date", "choice", "integer", "constant")


def field_operation(rule):
    return next((operation for operation in OPERATIONS if operation in rule), None)


def shape(rule):
    match field_operation(rule):
        case "text":
            return {"type": "string", "pattern": r"\S"}
        case "date":
            return {"type": "string", "format": "date"}
        case "choice":
            return {"enum": list(rule["choice"])}
        case "integer":
            constraint = {"type": "integer", "minimum": 0}
            if "maximum" in rule:
                constraint["maximum"] = rule["maximum"]
            return constraint
        case "constant":
            return {"const": rule["constant"]}
        case _:
            return {
                "type": "object",
                "properties": {name: shape(child) for name, child in rule.items()},
                "required": list(rule),
                "additionalProperties": False,
            }


def profile(kind, schedule, mapping):
    return {**mapping["common"], "work_schedule": {"kind": {"constant": kind}, **schedule}}


def schema(mapping=MAPPING):
    return {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "就労証明書の就労データ試作 v" + str(mapping["version"]),
        "oneOf": [shape(profile(kind, fields, mapping)) for kind, fields in mapping["schedules"].items()],
    }


def read(rule, cells):
    def integer(ref, maximum=None):
        value = cells.get(ref)
        # XLSX readers may return integral values as floats.
        if type(value) is float and value.is_integer():
            value = int(value)
        if type(value) is not int or value < 0 or (maximum is not None and value > maximum):
            raise ValueError(f"{ref}: a valid nonnegative integer is required")
        return value

    match field_operation(rule):
        case "text":
            ref = rule["text"]
            value = cells.get(ref)
            if not isinstance(value, str) or not value.strip():
                raise ValueError(f"{ref}: nonempty text is required")
            return value
        case "date":
            return date(*(integer(ref) for ref in rule["date"])).isoformat()
        case "choice":
            options = rule["choice"]
            if any(cells.get(ref) not in (None, "", "□") for ref in rule.get("unsupported", ())):
                raise ValueError("An unsupported employment option is filled")
            selected = [name for name, ref in options.items() if cells.get(ref) == "☑"]
            if len(selected) != 1 or any(cells.get(ref) not in (None, "", "□", "☑") for ref in options.values()):
                raise ValueError(f"{', '.join(options.values())}: select exactly one supported option")
            return selected[0]
        case "integer":
            return integer(rule["integer"], rule.get("maximum"))
        case "constant":
            return rule["constant"]
        case _:
            return {name: read(child, cells) for name, child in rule.items()}


def schedule_is_filled(rule, cells):
    match field_operation(rule):
        case "integer":
            return cells.get(rule["integer"]) not in (None, "")
        case "choice":
            return any(cells.get(ref) not in (None, "", "□") for ref in rule["choice"].values())
        case "constant":
            return False
        case _:
            return any(schedule_is_filled(child, cells) for child in rule.values())


def extract(cells, mapping=MAPPING):
    if not isinstance(cells, dict):
        raise ValueError("Expected a JSON object mapping A1 addresses to cell values")
    selected = [(kind, fields) for kind, fields in mapping["schedules"].items() if schedule_is_filled(fields, cells)]
    if len(selected) != 1:
        raise ValueError("Exactly one of the fixed and variable schedule sections must be filled")
    return read(profile(*selected[0], mapping), cells)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("schema", "extract"))
    parser.add_argument("cells", type=Path, nargs="?")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--approved-mapping", type=Path)
    source.add_argument("--okf", type=Path)
    args = parser.parse_args()
    if (args.command == "extract") != (args.cells is not None):
        parser.error("extract requires a JSON cell map, schema takes no input file")
    try:
        mapping = MAPPING
        if args.approved_mapping is not None:
            from review import load_approved

            mapping = load_approved(args.approved_mapping)
        elif args.okf is not None:
            from okf import load_concept

            mapping = load_concept(args.okf)
        result = (
            schema(mapping) if args.cells is None
            else extract(json.loads(args.cells.read_text(encoding="utf-8")), mapping)
        )
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
