#!/usr/bin/env python3
"""Capture basic diagnostic text from the pinned Harunobu implementation."""
import json, subprocess, sys
from pathlib import Path
repo = Path(sys.argv[1]).resolve()
commit = '88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip() != commit:
    raise SystemExit('Harunobu checkout is not at the pinned revision')
code = r'''
import json, random, os
os.environ['HARUNOBU_AI_DISABLED']='1'
from types import SimpleNamespace as S
from harunobu.core.models import MergedRange, CellRange
from harunobu.rules.level1.L1_12_merged_cells import MergedCellsRule
from harunobu.rules.level1.L1_01_file_format import FileFormatRule
from harunobu.rules.level1.L1_02_one_table_per_sheet import OneTablePerSheetRule
from harunobu.rules.level1.L1_13_hidden_rows_columns import HiddenRowsColumnsRule
from harunobu.rules.registry import registry
from harunobu.rules.level3.L3_01_header_start_position import HeaderStartPositionRule
from harunobu.rules.level2.L2_06_formula_to_value import FormulaToValueRule
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
tables_cases=[]
for i in range(120):
    ranges=[[r,0,r+1,2] for r in range(0,rng.randrange(1,5)*3,3)]
    confidences=[rng.choice([0.2,0.49,0.5,0.8,1.0]) for _ in ranges]
    tables=[S(range=CellRange(start_row=r+1,start_col=c+1,end_row=rr+1,end_col=cc+1),confidence=confidence) for (r,c,rr,cc),confidence in zip(ranges,confidences)]
    current=rng.randrange(len(tables))
    result=OneTablePerSheetRule().check(S(all_tables=tables,table_region=tables[current],sheet=S(name='表 1')))
    tables_cases.append(dict(ranges=ranges,confidences=confidences,current=current,message=result.message,descriptions=[v.description for v in result.violations]))
start_cases=[]
for i in range(140):
    first=rng.randrange(8); headers=[r for r in range(12) if rng.randrange(4)==0]; body=rng.randrange(14)
    region=S(range=CellRange(start_row=first+1,start_col=1,end_row=14,end_col=4),layout=S(header_rows=[r+1 for r in headers],body_start_row=body+1))
    result=HeaderStartPositionRule().check(S(table_region=region,sheet=S(name='表')))
    start_cases.append(dict(first=first,headers=headers,body=body,message=result.message,descriptions=[v.description for v in result.violations]))
formula_cases=[]
for i in range(120):
    refs=[]
    for j in range(rng.randrange(8)):
        row,col=rng.randrange(7),rng.randrange(4)
        refs.append(dict(row=row,col=col,cell_ref=f"{chr(65+col)}{row+1}",formula_text=rng.choice([None,'','1+1','=SUM(A1:A2)','式' * 52]),formula_type=rng.choice([None,'','shared','array'])))
    errors=[]
    for j in range(rng.randrange(4)):
        row,col=rng.randrange(7),rng.randrange(4)
        errors.append(dict(row=row,col=col,cell_ref=f"{chr(65+col)}{row+1}"))
    formula_refs=[S(**{**r,'row':r['row']+1,'col':r['col']+1}) for r in refs]
    formula_errors=[S(**{**r,'row':r['row']+1,'col':r['col']+1,'error_text':'#REF!'}) for r in errors]
    sheet=S(name='表',formula_refs_loaded=True,formula_refs=formula_refs,formula_error_refs=formula_errors)
    result=FormulaToValueRule().check(S(sheet=sheet,table_region=S(range=CellRange(start_row=2,start_col=2,end_row=6,end_col=3))))
    formula_cases.append(dict(refs=refs,errors=errors,message=result.message,descriptions=[v.description for v in result.violations],locations=[v.cell_range for v in result.violations]))
registry.discover()
skipped=[]
for rule_id in ['L1-09','L3-02','L3-03','L3-07','L3-08']:
    result=registry.get(rule_id).check(None)
    skipped.append(dict(id=rule_id,message=result.message,confidence=result.confidence,passed=result.passed))
print(json.dumps(dict(cases=cases,formats=formats,hidden=hidden,skipped=skipped,tables=tables_cases,start=start_cases,formula=formula_cases),ensure_ascii=False))
'''
data=json.loads(subprocess.check_output([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,text=True))
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-basic-diagnostics-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,**data},ensure_ascii=False,separators=(',',':'))+'\n')
