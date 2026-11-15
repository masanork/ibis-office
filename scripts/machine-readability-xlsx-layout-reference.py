#!/usr/bin/env python3
"""Capture XLSX reader and layout outcomes from pinned Harunobu."""
import json
from pathlib import Path
import subprocess
import sys
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent
code=r'''
import io,json,sys
from datetime import date,datetime,time
from pathlib import Path
from harunobu.core.reader import read_workbook
from harunobu.core.layout.detector import LayoutDetector
from harunobu.core.models import Config
out=[]
for name in ['structure.xlsx','header-merge.xlsx']:
    wb=read_workbook(name,io.BytesIO((Path(sys.argv[1])/name).read_bytes())); sheet=wb.sheets[0]
    cells=[]
    for (r,c),cell in sorted(sheet.cells.items()):
        value=cell.value
        if value is None: kind=None; text=None
        elif isinstance(value,(int,float)): kind='numeric';text=None
        elif isinstance(value,(date,datetime,time)):kind='date';text=None
        else:kind='text';text=str(value)
        cells.append(dict(row=r-1,column=c-1,kind=kind,text=text,bold=cell.fmt.bold,merged=cell.is_merged,formula=cell.formula))
    result=LayoutDetector(Config(mode='standard')).detect_with_others(sheet)
    out.append(dict(file=name,last_row=sheet.max_row-1,last_column=sheet.max_col-1,cells=cells,merges=[[m.start_row-1,m.start_col-1,m.end_row-1,m.end_col-1] for m in sheet.merged_cells],tables=[[t.range.start_row-1,t.range.start_col-1,t.range.end_row-1,t.range.end_col-1,t.confidence,[r-1 for r in t.layout.header_rows]] for t in result.tables],others=[[o.row-1,o.col-1,o.region_type] for o in result.others]))
print(json.dumps(out,ensure_ascii=False))
'''
fixtures=root/'test/fixtures/machine-readability'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code,str(fixtures)],text=True,capture_output=True,check=True)
(fixtures/'harunobu-xlsx-layout-reference.json').write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
