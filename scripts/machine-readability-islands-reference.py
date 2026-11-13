#!/usr/bin/env python3
"""Capture preliminary island detection from pinned Harunobu, before merges."""
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
from harunobu.core.layout.island import IslandDetector,_split_into_bands,_merge_header_body_bands
from harunobu.core.models import Cell,CellFormat,CellPosition,MergedRange,Sheet
rng=random.Random(79821)
values=[None,None,'','text','項目','合計','※ 注記','出典：例','123','1,000',1,True]
formulas=[None,None,None,'=SUM(A1:A2)','=SUMMARY(A1)']
cases=[]
for _ in range(400):
    height,width=rng.randrange(1,15),rng.randrange(1,13)
    raw=[]
    density=rng.choice([0.1,0.2,0.4,0.7])
    for r in range(height):
        for c in range(width):
            if rng.random()>=density: continue
            v=rng.choice(values)
            raw.append(dict(row=r,column=c,value=v,bold=rng.random()<0.1,merged=rng.random()<0.1,formula=rng.choice(formulas) if v is not None else None))
    mapped={(a['row']+1,a['column']+1):Cell(pos=CellPosition(row=a['row']+1,col=a['column']+1),value=a['value'],fmt=CellFormat(bold=a['bold']),is_merged=a['merged'],formula=a['formula']) for a in raw}
    merge_raw=[]
    for _ in range(rng.randrange(4)):
        r=rng.randrange(height);c=rng.randrange(width)
        merge_raw.append([r,c,rng.randrange(r,height),rng.randrange(c,width)])
    merges=[MergedRange(start_row=r+1,start_col=c+1,end_row=rr+1,end_col=cc+1) for r,c,rr,cc in merge_raw]
    sheet=Sheet(name='fixture',cells=mapped,merged_cells=merges,max_row=height,max_col=width)
    occupied_rows=sorted({r for (r,c),cell in mapped.items() if cell.value is not None})
    occupied_cols=sorted({c for (r,c),cell in mapped.items() if cell.value is not None})
    table=[];others=[];candidates=[]
    if occupied_rows and occupied_cols:
        rb=_merge_header_body_bands(sheet,_split_into_bands(occupied_rows,1),_split_into_bands(occupied_cols,1),1)
        cb=_split_into_bands(occupied_cols,1)
        for r in rb:
            for c in cb:
                region,extra=IslandDetector()._evaluate_region_with_trim(sheet,r,c)
                if region is not None: candidates.append(region)
                others.extend(extra)
        for region in candidates:
            if IslandDetector()._is_trivial(region):
                for r in range(region.range.start_row,region.range.end_row+1):
                    for c in range(region.range.start_col,region.range.end_col+1):
                        cell=sheet.get_cell(r,c)
                        if cell and cell.value is not None: others.append((r,c,'isolated'))
            else: table.append(region)
    encoded_others=[]
    for o in others:
        if isinstance(o,tuple): r,c,kind=o
        else: r,c,kind=o.row,o.col,o.region_type
        encoded_others.append([r-1,c-1,kind])
    cases.append(dict(height=height,width=width,cells=raw,merges=merge_raw,candidates=[[t.range.start_row-1,t.range.start_col-1,t.range.end_row-1,t.range.end_col-1,t.confidence] for t in table],others=encoded_others))
print(json.dumps(cases,ensure_ascii=False))
'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-islands-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
