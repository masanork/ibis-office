#!/usr/bin/env python3
"""Record unmerged CSV L2-03 outcomes from pinned Harunobu."""
import json, os, subprocess, sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'; repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.models import Cell,CellPosition,Sheet,WorkBook,TableContext,TableRegion,TableLayout,CellRange
from harunobu.rules.level2.L2_03_unique_headers import UniqueHeadersRule
rng=random.Random(105); cases=[]
patterns=[['1','2','3'],['001','002','003'],['1.0','2e0','+3'],['1_0','11','12'],['3','2','1'],['1','2','名称'],['a','','c'],['A','A','B'],['A','B','A'],['','','']]
for i in range(160):
    cols=rng.randint(2,5); rows=rng.randint(2,6); h=rng.choice([[],[0],[0,1]])
    values=['',' ','項目','2','01','-2','2.5','A','\u3000']; cells=[]
    for r in range(rows):
        for c in range(cols):
            if rng.random()<.68: cells.append([r,c,rng.choice(values)])
    if i<len(patterns):
        cols=3; rows=4; h=[0]; cells=[[0,c,v] for c,v in enumerate(patterns[i])]+[[r,c,'値'] for r in range(1,4) for c in range(3)]
    body_first=max(h)+1 if h else 0; body_last=rows-1
    mapping={(r+1,c+1):Cell(pos=CellPosition(row=r+1,col=c+1),value=v) for r,c,v in cells}
    sheet=Sheet(name='CSV',cells=mapping,max_row=rows,max_col=cols)
    workbook=WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
    region=TableRegion(range=CellRange(start_row=1,start_col=1,end_row=rows,end_col=cols),layout=TableLayout(header_rows=[r+1 for r in h],body_start_row=body_first+1,body_end_row=body_last+1),confidence=1)
    result=UniqueHeadersRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region))
    cases.append(dict(range=[0,0,rows-1,cols-1],headers=h,body=[body_first,body_last],cells=cells,passed=result.passed,confidence=result.confidence,locations=[v.cell_range for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-unique-headers-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
