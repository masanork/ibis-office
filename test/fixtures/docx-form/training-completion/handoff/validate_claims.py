#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Validate the training-completion pilot's unsigned claims handoff."""

import argparse
from datetime import date
import json
from pathlib import Path
import re

FIELDS = {"training", "completion_date", "number"}


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"項目が重複しています（{key}）")
        result[key] = value
    return result


def validate(claims):
    if not isinstance(claims, dict):
        raise ValueError("JSONオブジェクトを指定してください")

    missing = FIELDS - claims.keys()
    if missing:
        raise ValueError(f"必須項目がありません（{', '.join(sorted(missing))}）")
    extra = claims.keys() - FIELDS
    if extra:
        raise ValueError(f"定義されていない項目があります（{', '.join(sorted(extra))}）")

    training = claims["training"]
    if not isinstance(training, str) or not training.strip():
        raise ValueError("training は空白以外の文字を含む文字列にしてください")

    completed = claims["completion_date"]
    if not isinstance(completed, str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", completed):
        raise ValueError("completion_date は YYYY-MM-DD 形式の文字列にしてください")
    try:
        date.fromisoformat(completed)
    except ValueError as error:
        raise ValueError("completion_date は実在する日付にしてください") from error

    number = claims["number"]
    if not isinstance(number, str) or not re.fullmatch(r"[0-9]{5}", number):
        raise ValueError("number は半角数字5桁の文字列にしてください")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("claims", type=Path)
    args = parser.parse_args()
    try:
        claims = json.loads(args.claims.read_text(encoding="utf-8"), object_pairs_hook=unique_object)
        validate(claims)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print("研修修了証の受け渡し形式に適合しています")


if __name__ == "__main__":
    main()
