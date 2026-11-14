#!/usr/bin/env python3
"""Capture L1-03 with all unmerged CSV tables from pinned Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.models import Cell,CellPosition,Sheet,WorkBook,TableContext,TableRegion,CellRange
from harunobu.rules.level1.L1_03_data_fragmentation import DataFragmentationRule
rng=random.Random(1033);cases=[]
for i in range(160):
    tables=[]
    for _ in range(rng.randint(1,4)):
        r=rng.randint(0,12);c=rng.randint(0,4);tables.append([r,c,min(r+rng.randint(0,4),15),min(c+rng.randint(0,3),7)])
    cells=[]
    for r in range(16):
        for c in range(8):
            if rng.random()<.15: cells.append([r,c,rng.choice([' ','値','0','\u3000'])])
    mapping={(r+1,c+1):Cell(pos=CellPosition(row=r+1,col=c+1),value=v) for r,c,v in cells}
    sheet=Sheet(name='CSV',cells=mapping,max_row=16,max_col=8);workbook=WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
    regions=[TableRegion(range=CellRange(start_row=a+1,start_col=b+1,end_row=c+1,end_col=d+1),confidence=.9) for a,b,c,d in tables]
    for index,region in enumerate(regions):
        result=DataFragmentationRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region,all_tables=regions))
        cases.append(dict(tables=tables,current=index,cells=cells,passed=result.passed,confidence=result.confidence,locations=[v.cell_range for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-multitable-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
