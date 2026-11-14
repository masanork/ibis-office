#!/usr/bin/env python3
"""Capture L3-05 on unmerged CSV cells from pinned Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.models import Cell,CellPosition,Sheet,WorkBook,TableContext,TableRegion,TableLayout,CellRange
from harunobu.rules.level3.L3_05_time_axis_standardization import TimeAxisStandardizationRule
rng=random.Random(305);cases=[]
headers=['年月日','period','対象期間','場所','年','令和3年','R8.05.13','２０２４','']
values=['','2024-01-01','2024-01-01T15:30Z','2023年5月','2022/2/1','2021年度','R8.05.13','令和3年5月','03/04/01','令和3','日本',' 2024/1 ', '令和3年','2024-01-02T10:30:00+09:00']
for i in range(160):
    cols=rng.randint(1,5);rows=rng.randint(2,8);h=[] if i%9==0 else [0];cells=[]
    for r in range(rows):
        for c in range(cols):
            if rng.random()<.9: cells.append([r,c,rng.choice(headers if r==0 else values)])
    mapping={(r+1,c+1):Cell(pos=CellPosition(row=r+1,col=c+1),value=v) for r,c,v in cells}
    sheet=Sheet(name='CSV',cells=mapping,max_row=rows,max_col=cols);workbook=WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
    region=TableRegion(range=CellRange(start_row=1,start_col=1,end_row=rows,end_col=cols),layout=TableLayout(header_rows=[v+1 for v in h],body_start_row=2,body_end_row=rows),confidence=1)
    result=TimeAxisStandardizationRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region))
    cases.append(dict(rows=rows,cols=cols,headers=h,cells=cells,passed=result.passed,confidence=result.confidence,message=result.message,descriptions=[v.description for v in result.violations],findings=[[v.cell_range,v.severity] for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-time-axis-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
