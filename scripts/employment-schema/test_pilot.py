# SPDX-License-Identifier: AGPL-3.0-only
import json
from pathlib import Path
import tempfile
import unittest

from jsonschema import Draft202012Validator

from pilot import extract, schema
from review import load_approved, reviewed_mapping

COMMON = {"Z4": "架空事業所", "I19": "架空従業員", "AA3": 2026, "AF3": 11, "AI3": 23}
FIXED = {**COMMON, "I23": "☑", "W25": 180, "AA25": 0, "AG25": 1200, "Q27": 20, "AC27": 5}
VARIABLE = {
    **COMMON, "L23": "☑", "M31": "☑", "P31": "□", "M32": "□", "P32": "☑",
    "S31": 96, "W31": 30, "AC31": 480, "S32": 4,
}


class EmploymentSchemaTest(unittest.TestCase):
    def test_human_review_and_approved_mapping(self):
        draft = json.loads(Path(__file__).with_name("candidate.example.json").read_text(encoding="utf-8"))
        with self.assertRaisesRegex(ValueError, "approval pending"):
            reviewed_mapping(draft, "Test reviewer")
        for check in draft["checks"].values():
            check.update(decision="accepted", review_note="Test decision")
        with self.assertRaisesRegex(ValueError, "already assigned"):
            reviewed_mapping(draft, "Test reviewer")
        draft["mapping"]["schedules"]["variable"]["days"]["unit"]["choice"] = {"month": "M32", "week": "P32"}
        draft["mapping"]["common"]["employee_name"]["text"] = "I52"
        with self.assertRaisesRegex(ValueError, "Outside the company section"):
            reviewed_mapping(draft, "Test reviewer")
        draft["mapping"]["common"]["employee_name"]["text"] = "I19"
        approved = reviewed_mapping(draft, "Test reviewer")
        with tempfile.TemporaryDirectory() as directory:
            file = Path(directory) / "mapping.json"
            file.write_text(json.dumps(draft), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "An approved mapping is required"):
                load_approved(file)
            file.write_text(json.dumps(approved), encoding="utf-8")
            mapping = load_approved(file)
            self.assertEqual(schema(mapping), schema())
            self.assertEqual(extract(VARIABLE, mapping), extract(VARIABLE))
            approved["mapping"]["common"]["employee_name"]["text"] = "I18"
            file.write_text(json.dumps(approved), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "changed after approval"):
                load_approved(file)

    def test_schema_is_reproducible(self):
        checked_in = Path(__file__).with_name("employment.schema.json").read_text(encoding="utf-8")
        self.assertEqual(json.dumps(schema(), ensure_ascii=False, indent=2) + "\n", checked_in)
        Draft202012Validator.check_schema(schema())

    def test_fixed_variable_and_parent_fields(self):
        validator = Draft202012Validator(schema(), format_checker=Draft202012Validator.FORMAT_CHECKER)
        for source in (FIXED, VARIABLE):
            base = extract(source)
            self.assertEqual(base, extract({**source, "I52": "架空児童", "W52": "架空施設"}))
            validator.validate(base)
        self.assertEqual(extract(FIXED)["work_schedule"]["total"]["minutes"], 0)
        work = extract(VARIABLE)["work_schedule"]
        self.assertEqual(work["total"]["unit"], "month")
        self.assertEqual(work["days"], {"unit": "week", "count": 4})

    def test_ambiguous_or_invalid_input_requires_review(self):
        for cells in (
            {**FIXED, "W25": None},
            {**FIXED, "AF3": 2, "AI3": 30},
            {**FIXED, "S31": 96},
            {**FIXED, "Q23": "☑"},
            {**VARIABLE, "M32": "☑"},
            {**VARIABLE, "P32": "□"},
        ):
            with self.subTest(cells=cells), self.assertRaises(ValueError):
                extract(cells)


if __name__ == "__main__":
    unittest.main()
