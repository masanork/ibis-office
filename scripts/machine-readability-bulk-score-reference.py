#!/usr/bin/env python3
"""Capture reference bulk weighting and rounding on synthetic rule outcomes."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from types import SimpleNamespace as S
from harunobu.core.models import CheckResult
from harunobu.rules.bulk_base import BulkAnalysisResult
rng=random.Random(7307);cases=[]
for i in range(220):
    files=[];models=[]
    for fi in range(rng.randrange(0,6)):
        levels=[];mr={}
        for level in (1,2,3):
            rules=[];mapping={}
            for ri in range(rng.randrange(0,6)):
                r=dict(rule_id=f'L{level}-{ri:02d}',passed=rng.choice([True,False]),confidence=rng.choice([0.0,0.4,0.8,1.0]),severity=rng.choice(['fatal','critical','major','minor','info']))
                rules.append(r);mapping[r['rule_id']]=CheckResult(**r)
            levels.append(rules);mr[f'level{level}']=mapping
        files.append(levels);models.append(S(sheets=[S(tables=[S(mr_result=S(**mr))])]))
    checks=[];bulk={}
    for ri in range(rng.randrange(0,4)):
        r=dict(rule_id=f'bulk-{ri}',passed=rng.choice([True,False]),confidence=rng.choice([0.0,0.5,1.0]),severity='critical')
        checks.append(r);bulk[r['rule_id']]=CheckResult(**r)
    result=BulkAnalysisResult.model_construct(files=models,bulk_checks=bulk)
    cases.append(dict(files=files,checks=checks,per_file=result.per_file_scores(),total=result.total_score))
print(json.dumps(cases))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-bulk-score-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},separators=(',',':'))+'\n')
