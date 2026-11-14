#!/usr/bin/env python3
"""Capture report serialization from the pinned Harunobu implementation."""
import json, subprocess, sys
from pathlib import Path
repo = Path(sys.argv[1]).resolve()
commit = '88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip() != commit:
    raise SystemExit('Harunobu checkout is not at the pinned revision')
code = r'''
import json, random
from harunobu.core.models import CheckResult, Violation
from harunobu.core.severity import Severity
from harunobu.output.json_writer import _check_to_dict, _level_score_to_dict
from harunobu.core.scorer import LevelScorer
rng = random.Random(381)
cases = []
for i in range(100):
    rule_id = rng.choice(['L1-01','L1-12','L2-03','L3-07','L9-99'])
    raw = [dict(sheet=rng.choice(['表1','表2','(bulk)']),cell_range=f'A{j+1}',description=f'違反{j}',severity=rng.choice(['error','warning','info'])) for j in range(rng.randrange(0,42))]
    severity = rng.choice([None,*list(Severity)])
    result = CheckResult(rule_id=rule_id,passed=rng.choice([True,False]),confidence=rng.choice([0,0.5,1]),severity=severity,violations=[Violation(**v) for v in raw],message=f'ケース{i}')
    cases.append(dict(rule_id=rule_id,passed=result.passed,confidence=result.confidence,severity=severity.value if severity else None,findings=raw,message=result.message,expected=_check_to_dict(rule_id,result,include_original_description=False)))
scores = []
for level in range(1, 4):
    for start in range(0, len(cases), 4):
        selected = cases[start:start+4] if start else []
        checks = [CheckResult(rule_id=c['rule_id'],passed=c['passed'],confidence=c['confidence'],severity=c['severity']) for c in selected]
        scores.append(dict(level=level, inputs=list(range(start, min(start+4, len(cases)))) if start else [], expected=_level_score_to_dict(LevelScorer()._score_level(level,checks))))
print(json.dumps(dict(cases=cases,scores=scores),ensure_ascii=False))
'''
data = json.loads(subprocess.check_output([str(repo/'.venv/bin/python'), '-I','-X','utf8','-c',code],cwd=repo,text=True))
out = Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-report-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,**data},ensure_ascii=False,separators=(',',':'))+'\n')
