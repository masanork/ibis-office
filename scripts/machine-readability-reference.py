#!/usr/bin/env python3
"""Record selected reference results from an explicitly pinned Harunobu checkout."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

COMMIT = "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f"
ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "test/fixtures/machine-readability"
repo = Path(sys.argv[1]).resolve()
head = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
if head != COMMIT:
    raise SystemExit(f"Expected Harunobu {COMMIT}; got {head}")
result = {"harunobu_commit": COMMIT, "harunobu_version": "0.2.2", "mode": "standard", "ai_disabled": True, "fixtures": {}}
with tempfile.TemporaryDirectory() as tmp:
    for key, file in [("structure", "structure.xlsx"), ("csv", "data.csv"), ("header", "header-merge.xlsx")]:
        out = Path(tmp) / "result.json"
        env = {**os.environ, "HARUNOBU_AI_DISABLED": "1"}
        subprocess.run([str(repo / ".venv/bin/harunobu"), "analyze", str(FIXTURES / file), "--mode", "standard", "--output", "json", "--out-path", str(out)], check=True, env=env)
        data = json.loads(out.read_text())
        rules = []
        for sheet in data["sheets"]:
            for table in sheet["評価対象エリア"]:
                for rule in table["評価内容"]:
                    if rule["ルールID"] in {"L1-12", "L1-13", "L2-06"}:
                        rules.append({"sheet": sheet["シート名"], "table": table["範囲"], "rule": rule["ルールID"], "passed": rule["合否"], "skipped": rule["判定対象外"], "severity": rule["重大度"], "locations": [v["セル範囲"] for v in rule["違反"]], "finding_severities": [v["違反重大度"] for v in rule["違反"]]})
        result["fixtures"][key] = {"format": data["フォーマット"], "rules": sorted(rules, key=lambda r: r["rule"])}
(FIXTURES / "harunobu-reference.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")

# Character detection is intentionally compared separately from table inference
# and file encoding, because the live editor examines the whole sheet.
character_cases = ["abc日本語123", "①", "⑳", "⓪", "㉑", "㊿", "Ⅰ-0", "ⅿ", "Ａ", "ｚ", "９", "㈱", "㋿", "〜", "～", "㍻", "︰", "㎡", "㎏㎥", "面積㎡／Ａ", "Ⓐ", "😀"]
character_script = """import json, sys
from harunobu.rules.level1.L1_08_platform_dependent_chars import PlatformDependentCharsRule
r = PlatformDependentCharsRule()
print(json.dumps([{'value': value, 'detected': bool(r._detect_in_cell(value))} for value in json.load(sys.stdin)], ensure_ascii=False))
"""
process = subprocess.run([str(repo / ".venv/bin/python"), "-I", "-X", "utf8", "-c", character_script],
    input=json.dumps(character_cases, ensure_ascii=False), text=True, capture_output=True, check=True)
characters = {"harunobu_commit": COMMIT, "cases": json.loads(process.stdout)}
(FIXTURES / "harunobu-character-reference.json").write_text(json.dumps(characters, ensure_ascii=False, indent=2) + "\n")

# Score aggregation is compared independently from detection and inference.
scoring_script = """import json
from harunobu.core.models import CheckResult
from harunobu.core.scorer import LevelScorer

def rule(rule_id, passed, severity=None, confidence=1.0):
    return dict(rule_id=rule_id, passed=passed, severity=severity, confidence=confidence)
patterns = [
    [], [rule('L1-01', True)], [rule('L1-01', False, 'fatal')],
    [rule('skip', False, 'fatal', 0.0)],
    [rule('a', False, 'critical'), rule('b', False, 'major'), rule('c', True, 'minor')],
    [rule('a', False, None), rule('b', False, 'info'), rule('skip', True, 'fatal', 0.0)],
    [rule('repeated', False, 'critical')] * 11,
    [rule('a', False, 'fatal'), rule('a', False, 'major'), rule('b', True, 'fatal')],
]
scorer = LevelScorer()
cases = []
for level in (1, 2, 3):
    for rules in patterns:
        result = scorer._score_level(level, [CheckResult(**r) for r in rules])
        cases.append(dict(level=level, rules=rules, expected=result.model_dump(mode='json')))
print(json.dumps(cases, ensure_ascii=False))
"""
process = subprocess.run([str(repo / ".venv/bin/python"), "-I", "-X", "utf8", "-c", scoring_script],
    text=True, capture_output=True, check=True)
scoring = {"harunobu_commit": COMMIT, "cases": json.loads(process.stdout)}
(FIXTURES / "harunobu-scoring-reference.json").write_text(json.dumps(scoring, ensure_ascii=False, indent=2) + "\n")
