#!/usr/bin/env python3
"""Capture CSV raw diagnostics report text from pinned Harunobu."""
import json, subprocess, sys
from pathlib import Path
repo=Path(sys.argv[1]).resolve(); commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability'
cases=json.loads((root/'harunobu-csv-diagnostics-reference.json').read_text())['cases']
code=r'''
import json,sys
from harunobu.core.reader import _build_csv_diagnostics
from harunobu.core.models import Sheet,WorkBook,TableContext,TableRegion,CellRange
from harunobu.rules.level1.L1_14_csv_single_line import CsvSingleLineRule
from harunobu.rules.level1.L1_15_csv_quoting import CsvQuotingRule
out=[]
for c in json.load(sys.stdin):
    diag=_build_csv_diagnostics(c['text'],c['delimiter']);sheet=Sheet(name='CSV',csv_diagnostics=diag)
    workbook=WorkBook(file_name='data.csv',file_format='csv' if c['delimiter']==',' else 'tsv',sheets=[sheet])
    region=TableRegion(range=CellRange(start_row=1,start_col=1,end_row=2000,end_col=4),confidence=1.0)
    context=TableContext(workbook=workbook,sheet=sheet,table_region=region)
    results=[dict(message=r.message,descriptions=[v.description for v in r.violations]) for r in (CsvSingleLineRule().check(context),CsvQuotingRule().check(context))]
    out.append(dict(name=c['name'],results=results))
print(json.dumps(out,ensure_ascii=False))
'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,input=json.dumps(cases,ensure_ascii=False),capture_output=True,text=True,check=True)
(root/'harunobu-csv-report-reference.json').write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
