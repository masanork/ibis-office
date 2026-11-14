#!/usr/bin/env python3
"""Capture AI-disabled L3-09 on unmerged CSV headers."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.models import Cell,CellPosition,Sheet,WorkBook,TableContext,TableRegion,TableLayout,CellRange
from harunobu.rules.level3.L3_09_vertical_format import VerticalFormatRule
rng=random.Random(309);cases=[]
values=['2021','2022年度','2023年','1月','2月','3月','第1期','第2期','第3期','令和元年','R01','H30','件数1','件数2','件数3','件数4','名前','',' ', '１']
for i in range(160):
    cols=101 if i==1 else rng.randint(1,14); headers=[] if i%11==0 else [0];cells=[]
    for c in range(cols):
        if rng.random()<0.9: cells.append([0,c,rng.choice(values)])
    mapping={(r+1,c+1):Cell(pos=CellPosition(row=r+1,col=c+1),value=v) for r,c,v in cells}
    sheet=Sheet(name='CSV',cells=mapping,max_row=2,max_col=cols);workbook=WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
    region=TableRegion(range=CellRange(start_row=1,start_col=1,end_row=2,end_col=cols),layout=TableLayout(header_rows=[x+1 for x in headers],body_start_row=2,body_end_row=2),confidence=1)
    result=VerticalFormatRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region))
    cases.append(dict(cols=cols,headers=headers,cells=cells,passed=result.passed,confidence=result.confidence,message=result.message,descriptions=[v.description for v in result.violations],findings=[[v.cell_range,v.severity] for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-vertical-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
