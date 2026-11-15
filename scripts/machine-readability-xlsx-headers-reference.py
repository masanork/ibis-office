#!/usr/bin/env python3
"""Capture XLSX L1-05 on typed cells and merged headers."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.models import Cell,CellPosition,MergedRange,Sheet,WorkBook,TableContext,TableRegion,TableLayout,CellRange
from harunobu.rules.level1.L1_05_column_headers import ColumnHeadersRule
rng=random.Random(1105);cases=[]
for i in range(160):
    cols=rng.randint(2,5);rows=rng.randint(2,6);read_only=bool(i%2);h=[] if i%7==0 else [0]
    merges=[] if i%3==0 else [[0,0,0,1]]
    values=['',' ', '名前','値','001','2',1,2,3,1.5,True,False]
    cells=[];mapping={}
    for r in range(rows):
        for c in range(cols):
            if rng.random()<.75:
                v=rng.choice(values);cells.append([r,c,v])
                if not merges or read_only or (r,c)!=(0,1): mapping[(r+1,c+1)]=Cell(pos=CellPosition(row=r+1,col=c+1),value=v)
    for a,b,d,e in merges:
        for r in range(a,d+1):
            for c in range(b,e+1):
                cell=mapping.setdefault((r+1,c+1),Cell(pos=CellPosition(row=r+1,col=c+1)))
                cell.is_merged=True;cell.merge_master=CellPosition(row=a+1,col=b+1)
    sheet=Sheet(name='XLSX',cells=mapping,max_row=rows,max_col=cols,merged_cells=[MergedRange(start_row=a+1,start_col=b+1,end_row=d+1,end_col=e+1) for a,b,d,e in merges]);workbook=WorkBook(file_name='x.xlsx',file_format='xlsx',sheets=[sheet])
    region=TableRegion(range=CellRange(start_row=1,start_col=1,end_row=rows,end_col=cols),layout=TableLayout(header_rows=[r+1 for r in h],body_start_row=2,body_end_row=rows),confidence=1)
    result=ColumnHeadersRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region))
    cases.append(dict(rows=rows,cols=cols,headers=h,read_only=read_only,merges=merges,cells=cells,passed=result.passed,confidence=result.confidence,text_cells=[[r,c,str(v)] for r,c,v in cells],message=result.message,descriptions=[v.description for v in result.violations],locations=[v.cell_range for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-xlsx-headers-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
