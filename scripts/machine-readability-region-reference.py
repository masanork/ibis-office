#!/usr/bin/env python3
"""Capture AI-disabled L3-06 CSV outcomes from pinned Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.models import Cell,CellPosition,Sheet,WorkBook,TableContext,TableRegion,TableLayout,CellRange
from harunobu.rules.level3.L3_06_region_code import RegionCodeRule
rng=random.Random(306);cases=[]
headers=['地域','市区町村','都道府県コード','region code','調査地域','場所','名前','', ' ']
values=['','東京都','東京','青森県','青森','北海道','札幌市','札幌','京都市','京都','横浜市','横浜','大阪府','大阪','調査値','01','131016',' ','\u3000']
for i in range(160):
    cols=rng.randint(1,5);rows=rng.randint(2,8);h=[] if i%9==0 else [0];cells=[]
    for r in range(rows):
        for c in range(cols):
            if rng.random()<.9: cells.append([r,c,rng.choice(headers if r==0 else values)])
    mapping={(r+1,c+1):Cell(pos=CellPosition(row=r+1,col=c+1),value=v) for r,c,v in cells}
    sheet=Sheet(name='CSV',cells=mapping,max_row=rows,max_col=cols);workbook=WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
    region=TableRegion(range=CellRange(start_row=1,start_col=1,end_row=rows,end_col=cols),layout=TableLayout(header_rows=[v+1 for v in h],body_start_row=2,body_end_row=rows),confidence=1)
    result=RegionCodeRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region))
    cases.append(dict(rows=rows,cols=cols,headers=h,cells=cells,passed=result.passed,confidence=result.confidence,findings=[[v.cell_range,v.severity] for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-region-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
