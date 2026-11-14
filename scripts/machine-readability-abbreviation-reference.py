#!/usr/bin/env python3
"""Capture AI-disabled L2-02 on unmerged CSV cells from pinned Harunobu."""
import json, os, subprocess, sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.models import Cell,CellPosition,Sheet,WorkBook,TableContext,TableRegion,TableLayout,CellRange
from harunobu.rules.level2.L2_02_item_abbreviation import ItemAbbreviationRule
rng=random.Random(202); cases=[]
for i in range(180):
    cols=rng.randint(1,4); rows=rng.randint(1,8); stubs=[c for c in range(cols) if rng.random()<0.5]
    vals=['',' ','地域','〇','同上','同左','同右','〃','仝','ヽ','々','値','\u001c'];cells=[]
    for r in range(rows):
        for c in range(cols):
            if rng.random()<0.65: cells.append([r,c,rng.choice(vals)])
    mapping={(r+1,c+1):Cell(pos=CellPosition(row=r+1,col=c+1),value=v) for r,c,v in cells}
    sheet=Sheet(name='CSV',cells=mapping,max_row=rows,max_col=cols); workbook=WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
    region=TableRegion(range=CellRange(start_row=1,start_col=1,end_row=rows,end_col=cols),layout=TableLayout(stub_cols=[c+1 for c in stubs],body_start_row=1,body_end_row=rows),confidence=1)
    result=ItemAbbreviationRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region))
    cases.append(dict(rows=rows,stubs=stubs,cells=cells,passed=result.passed,confidence=result.confidence,findings=[[v.cell_range,v.severity] for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-abbreviation-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
