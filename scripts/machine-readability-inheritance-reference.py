#!/usr/bin/env python3
"""Capture header inheritance from the pinned Harunobu implementation."""
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
from harunobu.core.layout.island import IslandDetector
from harunobu.core.models import Cell,CellPosition,CellRange,ColumnHeader,ColumnSchema,Sheet,TableLayout,TableRegion
rng=random.Random(876554)
values=[None,'','N/A','data','12','1,234','2024-1-1',1,True]
types=['numeric','text','date','mixed','empty']
def pack(t):
    l=t.layout; r=t.range
    return dict(range=[r.start_row-1,r.start_col-1,r.end_row-1,r.end_col-1],confidence=t.confidence,abstain=t.abstain,
        rows=[x-1 for x in l.header_rows],body=[l.body_start_row-1,l.body_end_row-1],
        headers=[dict(column=h.col_index-1,rows=[x-1 for x in h.header_rows],label=h.label,merged=h.is_merged) for h in l.column_headers],
        schemas=[dict(column=x.col_index-1,kind=x.inferred_type,has_header=x.has_header) for x in l.columns],
        stub=[x-1 for x in l.stub_cols],footer=[x-1 for x in l.footer_rows])
cases=[]
for _ in range(300):
    width=rng.randrange(1,5); count=rng.randrange(1,7); start=1; tables=[]; cells={}; raw=[]
    for i in range(count):
        if i: start += rng.randrange(1,6)
        length=rng.randrange(1,4); end=start+length-1
        offset=rng.choice([0,0,0,1]); left=1+offset; right=left+width-1
        has_rows=rng.random()<0.3; has_headers=rng.random()<0.5; has_schemas=rng.random()<0.45
        headers=[ColumnHeader(col_index=c,header_rows=[start],label=rng.choice(['項目','',' ']),is_merged=rng.random()<0.25) for c in range(left,right+1)] if has_headers else []
        schemas=[ColumnSchema(col_index=c,inferred_type=rng.choice(types),has_header=rng.random()<0.5) for c in range(left,right+1)] if has_schemas else []
        layout=TableLayout(header_rows=[start] if has_rows else [],body_start_row=start,body_end_row=end,column_headers=headers,columns=schemas,stub_cols=[left] if rng.random()<0.3 else [],footer_rows=[end] if rng.random()<0.3 else [])
        tables.append(TableRegion(range=CellRange(start_row=start,start_col=left,end_row=end,end_col=right),layout=layout,confidence=rng.choice([0.4,0.7,1.0]),abstain=rng.random()<0.2))
        for r in range(start,end+1):
            for c in range(left,right+1):
                if rng.random()<0.35: continue
                v=rng.choice(values); cells[(r,c)]=Cell(pos=CellPosition(row=r,col=c),value=v)
                raw.append(dict(row=r-1,column=c-1,value=v))
        start=end
    rng.shuffle(tables)
    sheet=Sheet(name='fixture',cells=cells,max_row=start,max_col=width+1)
    result=IslandDetector()._inherit_headers_from_above(sheet,tables)
    cases.append(dict(cells=raw,tables=[pack(t) for t in tables],expected=[pack(t) for t in result]))
print(json.dumps(cases,ensure_ascii=False))
'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-inheritance-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
