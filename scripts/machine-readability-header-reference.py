#!/usr/bin/env python3
"""Record row-header signal strength from a pinned Harunobu checkout."""
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
from harunobu.core.layout.island import _header_row_strength, _band_is_header_like, _split_into_bands, _merge_header_body_bands, _estimate_layout
from harunobu.core.models import Cell, CellPosition, CellFormat, MergedRange, Sheet
rng = random.Random(70465)
values = [None, '', 'data', '1', '12', '123', '001', '-2', '+10', '1,000', '12.3', '３３３', '١٢٣', 'N/A', '×', '-', 'X', 0, True, 1.2]
cases=[]
for _ in range(400):
    width = rng.randrange(1, 13)
    cells=[]
    for c in range(width):
        if rng.random() < 0.22: continue
        cells.append(dict(row=0,column=c,value=rng.choice(values),bold=rng.random()<0.12,merged=rng.random()<0.35))
    merges=[]
    for _ in range(rng.randrange(5)):
        a=rng.randrange(width)
        b=rng.randrange(a,width)
        top=rng.randrange(0,2)
        bottom=rng.randrange(top,3)
        merges.append([top,a,bottom,b])
    mapped={(a['row']+1,a['column']+1): Cell(pos=CellPosition(row=a['row']+1,col=a['column']+1),value=a['value'],fmt=CellFormat(bold=a['bold']),is_merged=a['merged']) for a in cells}
    ranges=[MergedRange(start_row=r+1,start_col=c+1,end_row=rr+1,end_col=cc+1) for r,c,rr,cc in merges]
    sheet=Sheet(name='fixture',cells=mapped,merged_cells=ranges,max_row=3,max_col=width)
    cases.append(dict(width=width,cells=cells,merges=merges,strength=_header_row_strength(sheet,1,1,width)))
# Explicit horizontal, vertical, and incomplete-merge cases.
for merge_specs in [[(1,1,1,3)], [(1,1,3,1),(1,2,3,2),(1,3,3,3)], [(1,1,3,1),(1,2,3,2)]]:
    raw=[dict(row=0,column=c,value='項目',bold=False,merged=True) for c in range(3)]
    cells={(1,c+1):Cell(pos=CellPosition(row=1,col=c+1),value='項目',is_merged=True) for c in range(3)}
    merges=[MergedRange(start_row=r,start_col=c,end_row=rr,end_col=cc) for r,c,rr,cc in merge_specs]
    sheet=Sheet(name='fixture',cells=cells,merged_cells=merges,max_row=3,max_col=3)
    cases.append(dict(width=3,cells=raw,merges=[[r-1,c-1,rr-1,cc-1] for r,c,rr,cc in merge_specs],strength=_header_row_strength(sheet,1,1,3)))
bands=[]
for _ in range(300):
    height,width=rng.randrange(1,8),rng.randrange(1,10)
    raw=[]
    for r in range(height):
        for c in range(width):
            if rng.random() >= 0.45: continue
            raw.append(dict(row=r,column=c,value=rng.choice(values),bold=rng.random()<0.08,merged=rng.random()<0.2))
    mapped={(a['row']+1,a['column']+1):Cell(pos=CellPosition(row=a['row']+1,col=a['column']+1),value=a['value'],fmt=CellFormat(bold=a['bold']),is_merged=a['merged']) for a in raw}
    merges=[]
    for _ in range(rng.randrange(4)):
        r=rng.randrange(height);c=rng.randrange(width)
        merges.append([r,c,rng.randrange(r,height),rng.randrange(c,width)])
    ranges=[MergedRange(start_row=r+1,start_col=c+1,end_row=rr+1,end_col=cc+1) for r,c,rr,cc in merges]
    sheet=Sheet(name='fixture',cells=mapped,merged_cells=ranges,max_row=height,max_col=width)
    layout=_estimate_layout(sheet,1,height,1,width)
    bands.append(dict(height=height,width=width,cells=raw,merges=merges,header_like=_band_is_header_like(sheet,1,height,1,width),layout=[[r-1 for r in layout.header_rows],layout.body_start_row-1,layout.body_end_row-1]))
# Dense multi-row tables with headers, blanks, and numeric body values.
for _ in range(150):
    height,width=rng.randrange(2,19),rng.randrange(2,9)
    header_depth=rng.randrange(1,min(height,8)+1)
    raw=[]
    for r in range(height):
        for c in range(width):
            if rng.random()<0.2: continue
            if r<header_depth:
                value=rng.choice(['項目','区分','1','１２',None])
                bold=rng.random()<0.45
            else:
                value=rng.choice(['100','20','data',123,False,None])
                bold=False
            raw.append(dict(row=r,column=c,value=value,bold=bold,merged=False))
    merges=[]
    if rng.random()<0.5:
        top=rng.randrange(min(height,header_depth)); end=rng.randrange(top,height)
        column=rng.randrange(width)
        merges.append([top,column,end,column])
        for a in raw:
            if a['column']==column and top<=a['row']<=end: a['merged']=True
    mapped={(a['row']+1,a['column']+1):Cell(pos=CellPosition(row=a['row']+1,col=a['column']+1),value=a['value'],fmt=CellFormat(bold=a['bold']),is_merged=a['merged']) for a in raw}
    ranges=[MergedRange(start_row=r+1,start_col=c+1,end_row=rr+1,end_col=cc+1) for r,c,rr,cc in merges]
    sheet=Sheet(name='fixture',cells=mapped,merged_cells=ranges,max_row=height,max_col=width)
    layout=_estimate_layout(sheet,1,height,1,width)
    bands.append(dict(height=height,width=width,cells=raw,merges=merges,header_like=_band_is_header_like(sheet,1,height,1,width),layout=[[r-1 for r in layout.header_rows],layout.body_start_row-1,layout.body_end_row-1]))
merges_cases=[]
for band in bands:
    raw=band['cells']
    occupied=[a for a in raw if a['value'] is not None]
    rows=_split_into_bands(sorted({a['row']+1 for a in occupied}),1)
    cols=_split_into_bands(sorted({a['column']+1 for a in occupied}),1)
    mapped={(a['row']+1,a['column']+1):Cell(pos=CellPosition(row=a['row']+1,col=a['column']+1),value=a['value'],fmt=CellFormat(bold=a['bold']),is_merged=a['merged']) for a in raw}
    ranges=[MergedRange(start_row=r+1,start_col=c+1,end_row=rr+1,end_col=cc+1) for r,c,rr,cc in band['merges']]
    sheet=Sheet(name='fixture',cells=mapped,merged_cells=ranges,max_row=band['height'],max_col=band['width'])
    gap=rng.randrange(1,4)
    result=_merge_header_body_bands(sheet,rows,cols,gap)
    merges_cases.append(dict(**band,row_bands=[[a-1,b-1] for a,b in rows],column_bands=[[a-1,b-1] for a,b in cols],gap=gap,result=[[a-1,b-1] for a,b in result]))
# Explicit positive/negative cases and the large merged-header exception.
for height, spans, bold, with_merge in [(3,[(0,0),(2,2)],True,False), (3,[(0,0),(2,2)],False,False), (14,[(0,10),(12,13)],True,False), (14,[(0,10),(12,13)],True,True)]:
    raw=[dict(row=r,column=c,value='項目',bold=bold,merged=with_merge) for r in range(height) for c in range(3) if any(a<=r<=b for a,b in spans)]
    merged=[[0,0,10,0]] if with_merge else []
    mapped={(a['row']+1,a['column']+1):Cell(pos=CellPosition(row=a['row']+1,col=a['column']+1),value=a['value'],fmt=CellFormat(bold=a['bold']),is_merged=a['merged']) for a in raw}
    ranges=[MergedRange(start_row=r+1,start_col=c+1,end_row=rr+1,end_col=cc+1) for r,c,rr,cc in merged]
    sheet=Sheet(name='fixture',cells=mapped,merged_cells=ranges,max_row=height,max_col=3)
    result=_merge_header_body_bands(sheet,[(a+1,b+1) for a,b in spans],[(1,3)],1)
    merges_cases.append(dict(height=height,width=3,cells=raw,merges=merged,header_like=_band_is_header_like(sheet,1,height,1,3),row_bands=spans,column_bands=[[0,2]],gap=1,result=[[a-1,b-1] for a,b in result]))
print(json.dumps(dict(cases=cases,bands=bands,band_merges=merges_cases),ensure_ascii=False))
'''
run = subprocess.run([str(repo / '.venv/bin/python'), '-I', '-X', 'utf8', '-c', code], text=True, capture_output=True, check=True)
data = {'harunobu_commit': commit, **json.loads(run.stdout)}
out = Path(__file__).resolve().parent.parent / 'test/fixtures/machine-readability/harunobu-header-reference.json'
out.write_text(json.dumps(data, ensure_ascii=False, separators=(',', ':')) + '\n')
