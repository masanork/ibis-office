#!/usr/bin/env python3
"""Capture cross-file L3-07 outcomes from the pinned Harunobu implementation."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from types import SimpleNamespace as S
from harunobu.rules.level3.L3_07_format_consistency import FormatConsistencyRule
rng=random.Random(307);cases=[]
patterns=[[],[[]],[[],[]],[['A','B'],['A','B']],[['A','B'],['B','A']],[['A'],['A','B','C','D','E']],[['','A'],['','A']]]
for i in range(160):patterns.append([[rng.choice(['A','B','C','D','','地域','値']) for _ in range(rng.randrange(0,8))] for _ in range(rng.randrange(0,5))])
for pattern in patterns:
    entries=[]
    for i,headers in enumerate(pattern):
        # Supplied arrays already model the extraction's post-strip output.
        columns=[S(label=v) for v in headers]
        entries.append(S(file_name=f'f{i}.csv',analysis=S(sheets=[S(tables=[S(column_headers=columns)])])))
    normalized=[[v.strip() for v in headers if v] for headers in pattern]
    result=FormatConsistencyRule().check_bulk(entries)
    cases.append(dict(headers=normalized,passed=result.passed,confidence=result.confidence,severity=result.effective_severity.value,severities=[v.severity for v in result.violations],message=result.message,descriptions=[v.description for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-bulk-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
