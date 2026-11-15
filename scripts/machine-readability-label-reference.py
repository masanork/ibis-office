#!/usr/bin/env python3
"""Record generated column headers from a pinned local Harunobu checkout."""
import json
from pathlib import Path
import subprocess
import sys
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit:
    raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.layout.island import _generate_column_headers
from harunobu.core.models import Cell,CellPosition,Sheet
rng=random.Random(472310)
cases=[]
values=[None,None,'','  ','項目','区分','Ａ','1',1,True,'  名前  ','備考']
for _ in range(300):
    height,width=rng.randrange(1,8),rng.randrange(1,8)
    raw=[]; cells={}
    for r in range(height):
        for c in range(width):
            if rng.random()<0.35: continue
            value=rng.choice(values); merged=rng.random()<0.3
            master=[rng.randrange(height),rng.randrange(width)] if merged and rng.random()<0.65 else None
            raw.append(dict(row=r,column=c,value=None if value is None else str(value),merged=merged,master=master))
            cells[(r+1,c+1)]=Cell(pos=CellPosition(row=r+1,col=c+1),value=value,is_merged=merged,merge_master=None if master is None else CellPosition(row=master[0]+1,col=master[1]+1))
    rows=sorted(rng.sample(range(height),rng.randrange(height+1)))
    sheet=Sheet(name='fixture',cells=cells,max_row=height,max_col=width)
    result=_generate_column_headers(sheet,[r+1 for r in rows],1,width)
    cases.append(dict(width=width,rows=rows,cells=raw,headers=[dict(column=h.col_index-1,rows=[r-1 for r in h.header_rows],label=h.label,merged=h.is_merged) for h in result]))
print(json.dumps(cases,ensure_ascii=False))
'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-label-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
