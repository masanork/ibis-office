#!/usr/bin/env python3
"""Capture basic diagnostic text from the pinned Harunobu implementation."""
import json, subprocess, sys
from pathlib import Path
repo = Path(sys.argv[1]).resolve()
commit = '88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip() != commit:
    raise SystemExit('Harunobu checkout is not at the pinned revision')
code = r'''
import json, random
from types import SimpleNamespace as S
from harunobu.core.models import MergedRange, CellRange
from harunobu.rules.level1.L1_12_merged_cells import MergedCellsRule
from harunobu.rules.level1.L1_01_file_format import FileFormatRule
from harunobu.rules.level1.L1_13_hidden_rows_columns import HiddenRowsColumnsRule
rng=random.Random(112); cases=[]
for i in range(160):
    merges=[]
    for j in range(rng.randrange(7)):
        row,col=rng.randrange(8),rng.randrange(6)
        merges.append([row,col,min(row+rng.randrange(3),7),min(col+rng.randrange(3),5)])
    headers=[j for j in range(8) if rng.randrange(2)]
    sheet=S(name='表',merged_cells=[MergedRange(start_row=r+1,start_col=c+1,end_row=rr+1,end_col=cc+1) for r,c,rr,cc in merges])
    table=CellRange(start_row=2,start_col=2,end_row=6,end_col=5)
    context=S(workbook=S(file_format='xlsx'),sheet=sheet,table_region=S(range=table,layout=S(header_rows=[r+1 for r in headers])))
    result=MergedCellsRule().check(context)
    cases.append(dict(merges=merges,headers=headers,message=result.message,descriptions=[v.description for v in result.violations]))
formats=[]
for name in ['テスト.csv', 'a\'s.xlsx', '']:
    for fmt in ['xlsx','csv','tsv','xls','XLSX','']:
        result=FileFormatRule().check(S(workbook=S(file_format=fmt,file_name=name),sheet=S(name='表')))
        formats.append(dict(name=name,format=fmt,message=result.message,descriptions=[v.description for v in result.violations]))
hidden=[]
for i in range(140):
    rows=[r for r in range(14) if rng.randrange(4)==0]
    cols=[c for c in range(14) if rng.randrange(4)==0]
    max_row,max_col=rng.randrange(14),rng.randrange(14)
    result=HiddenRowsColumnsRule().check(S(workbook=S(file_format='xlsx'),sheet=S(name='表',hidden_rows=[r+1 for r in rows],hidden_cols=[c+1 for c in cols],max_row=max_row+1,max_col=max_col+1)))
    hidden.append(dict(rows=rows,cols=cols,max_row=max_row,max_col=max_col,message=result.message,descriptions=[v.description for v in result.violations]))
print(json.dumps(dict(cases=cases,formats=formats,hidden=hidden),ensure_ascii=False))
'''
data=json.loads(subprocess.check_output([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,text=True))
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-basic-diagnostics-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,**data},ensure_ascii=False,separators=(',',':'))+'\n')
