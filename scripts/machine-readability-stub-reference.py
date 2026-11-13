#!/usr/bin/env python3
"""Capture row-header column scores from pinned Harunobu."""
import json
from pathlib import Path
import subprocess
import sys
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.layout.island import _stub_col_score, _estimate_stub_cols
from harunobu.core.models import Cell,CellFormat,CellPosition,ColumnSchema,Sheet
rng=random.Random(992131)
values=[None,None,'text','01','1','2','0','0010',0,1,2,3,1.0,1.5,True,False]
types=['numeric','text','date','mixed','empty',None]
cases=[]
for _ in range(400):
    height,width=rng.randrange(1,11),rng.randrange(1,5);col=rng.randrange(width); kind=rng.choice(types)
    raw=[];mapped={}
    for r in range(height):
        for c in range(width):
            if rng.random()<0.35: continue
            value=rng.choice(values);bold=rng.random()<0.35;bg=rng.choice([None,None,'FFFFFFFF','FF123456']);left=rng.random()<0.3;right=rng.random()<0.3
            key=None
            if isinstance(value,int): key=int(value)
            elif isinstance(value,float) and value.is_integer(): key=int(value)
            elif isinstance(value,str) and value.strip().isdigit(): key=int(value.strip())
            key=str(key) if key is not None and key>0 else None
            raw.append(dict(row=r,column=c,present=value is not None,key=key,bold=bold,background_nonwhite=bg is not None and bg!='FFFFFFFF',left=left,right=right))
            mapped[(r+1,c+1)]=Cell(pos=CellPosition(row=r+1,col=c+1),value=value,fmt=CellFormat(bold=bold,bg_color=bg,border_left='thin' if left else None,border_right='thin' if right else None))
    body_first,body_last=(2,1) if rng.random()<0.1 else (1,height)
    sheet=Sheet(name='fixture',cells=mapped,max_row=height,max_col=width)
    schema={} if kind is None else {col+1:ColumnSchema(col_index=col+1,inferred_type=kind)}
    score=_stub_col_score(sheet,body_first,body_last,col+1,width,schema)
    stubs=_estimate_stub_cols(sheet,body_first,body_last,1,width,list(schema.values()))
    cases.append(dict(height=height,width=width,column=col,first=body_first-1,last=body_last-1,kind=kind,cells=raw,score=score,stubs=[s-1 for s in stubs]))
print(json.dumps(cases,ensure_ascii=False))
'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-stub-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
