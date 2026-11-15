#!/usr/bin/env python3
"""Record column schemas from a pinned local Harunobu checkout."""
import json
from pathlib import Path
import subprocess
import sys

commit = '88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo = Path(sys.argv[1]).resolve()
if subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip() != commit:
    raise SystemExit('Harunobu checkout is not at the pinned revision')
code = r'''
import json, random
from datetime import date
from harunobu.core.layout.island import _infer_column_schemas
from harunobu.core.models import Cell, CellPosition, ColumnHeader, Sheet
rng=random.Random(243880)
values=[None,None,'',' ', '-', '－','N/A','NA','×','x','data','１２','12.0','1,234','+2','abc','2024-12-31','30/2/2020','2024-2/31',0,True,1.2,{'date':'2024-01-01'}]
cases=[]
for _ in range(500):
    height=rng.randrange(1,18)
    raw=[]
    for r in range(height):
        if rng.random()<0.2: continue
        v=rng.choice(values)
        raw.append(dict(row=r,value=v))
    sheet=Sheet(name='fixture',cells={(a['row']+1,1):Cell(pos=CellPosition(row=a['row']+1,col=1),value=date(2024,1,1) if isinstance(a['value'],dict) else a['value']) for a in raw},max_row=height,max_col=1)
    label=rng.choice([None,'','項目',' '])
    headers=[] if label is None else [ColumnHeader(col_index=1,header_rows=[1],label=label)]
    schema=_infer_column_schemas(sheet,1,height,1,1,headers)[0]
    cases.append(dict(height=height,cells=raw,label=label,kind=schema.inferred_type,has_header=schema.has_header))
# Include threshold edges where exactly 80% share a type.
for values_case in [[1,2,3,4,'data'],[1,2,3,'data','data'],['2024/1/1']*4+['other'],['data']*4+[1],['x',None,'']]:
    raw=[dict(row=r,value=v) for r,v in enumerate(values_case)]
    sheet=Sheet(name='fixture',cells={(a['row']+1,1):Cell(pos=CellPosition(row=a['row']+1,col=1),value=a['value']) for a in raw},max_row=len(raw),max_col=1)
    schema=_infer_column_schemas(sheet,1,len(raw),1,1,[])[0]
    cases.append(dict(height=len(raw),cells=raw,label=None,kind=schema.inferred_type,has_header=schema.has_header))
print(json.dumps(cases,ensure_ascii=False))
'''
run = subprocess.run([str(repo / '.venv/bin/python'), '-I', '-X', 'utf8', '-c', code], text=True, capture_output=True, check=True)
data={'harunobu_commit':commit,'cases':json.loads(run.stdout)}
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-column-reference.json'
out.write_text(json.dumps(data,ensure_ascii=False,separators=(',',':'))+'\n')
