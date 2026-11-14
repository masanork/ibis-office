#!/usr/bin/env python3
"""Capture L3-01 and always-skipped rule results from pinned Harunobu."""
import json, os, subprocess, sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json, random
from harunobu.core.models import Sheet, WorkBook, TableContext, TableRegion, TableLayout, CellRange
from harunobu.rules.level3.L3_01_header_start_position import HeaderStartPositionRule
from harunobu.rules.level3.L3_02_code_table import CodeTableRule
from harunobu.rules.level3.L3_03_special_symbols import SpecialSymbolsRule
from harunobu.rules.level3.L3_08_metadata import MetadataRule
rng=random.Random(301); cases=[]; sheet=Sheet(name='Sheet'); workbook=WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
for i in range(180):
    start=rng.randrange(0,5); end=start+rng.randrange(1,9); headers=sorted(rng.sample(range(start,end+1),rng.randrange(0,min(5,end-start+2))))
    body=rng.randrange(start,end+2)
    region=TableRegion(range=CellRange(start_row=start+1,start_col=1,end_row=end+1,end_col=3),layout=TableLayout(header_rows=[v+1 for v in headers],body_start_row=body+1,body_end_row=end+1))
    context=TableContext(workbook=workbook,sheet=sheet,table_region=region)
    checks=[]
    for rule in [HeaderStartPositionRule(),CodeTableRule(),SpecialSymbolsRule(),MetadataRule()]:
        result=rule.check(context)
        checks.append(dict(id=rule.rule_id,passed=result.passed,confidence=result.confidence,severity=(result.severity or rule.severity).value,findings=[[v.cell_range,v.severity] for v in result.violations]))
    cases.append(dict(start=start,end=end,headers=headers,body=body,checks=checks))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-header-position-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
