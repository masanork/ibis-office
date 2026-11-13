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
                        rules.append({"sheet": sheet["シート名"], "table": table["範囲"], "rule": rule["ルールID"], "passed": rule["合否"], "skipped": rule["判定対象外"], "locations": [v["セル範囲"] for v in rule["違反"]]})
        result["fixtures"][key] = {"format": data["フォーマット"], "rules": sorted(rules, key=lambda r: r["rule"])}
(FIXTURES / "harunobu-reference.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
