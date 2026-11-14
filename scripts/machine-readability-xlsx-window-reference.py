#!/usr/bin/env python3
"""Generate synthetic XLSX window cases and pinned Harunobu expectations."""
import json
from pathlib import Path
import subprocess
import sys
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability'
code=r'''
import io,json,sys,zipfile
from pathlib import Path
from openpyxl import Workbook,load_workbook
from openpyxl.styles import Font
from harunobu.core.reader import _detect_effective_max_col,read_workbook
w=Workbook();w.remove(w.active)
w.create_sheet('empty')
s=w.create_sheet('simple');s['A1']='region';s['C2']='value'
s=w.create_sheet('wide_sample');s['A1']='region';s.cell(3,300,'far')
s=w.create_sheet('wide_late');s['A1']='region';s.cell(101,300,'late')
s=w.create_sheet('wide_formula');s['A1']='region';s.cell(1,300,'=1+1')
s=w.create_sheet('late_row');s['A1']='region';s.cell(10001,1,'late')
s=w.create_sheet('styled_blank');s['A1']='region';s.cell(200,300).font=Font(bold=True)
s=w.create_sheet('merged_wide');s['A1']='region';s.merge_cells(start_row=200,start_column=298,end_row=200,end_column=300)
s=w.create_sheet('bold_blank');s['A1']='value';s['B1'].font=Font(bold=True)
s=w.create_sheet('formats');s['A1']='header';s['B1']='value';s['A2']='date';s['B2']=45292;s['B2'].number_format='yyyy-mm-dd';s['A3']='number';s['B3']=123;s['B3'].number_format='#,##0';s['A4']='duration';s['B4']=1.5;s['B4'].number_format='[h]:mm:ss';s['A5']='builtin';s['B5']=45292;s['B5'].number_format='mm-dd-yy';s['A6']='percent';s['B6']=0.5;s['B6'].number_format='0.00%';s['A7']='bracket';s['B7']=1;s['B7'].number_format='[H]:MM';s['A8']='time';s['B8']=0.5;s['B8'].number_format='hh:mm:ss'
out=Path(sys.argv[1]);w.save(out)
r=load_workbook(out,data_only=True)
cases=[]
for i,ws in enumerate(r.worksheets):
    maxrow=ws.max_row or 0;maxcol=ws.max_column or 0
    rows=min(maxrow,10000);cols=min(maxcol,500)
    if maxcol>=256: cols=min(_detect_effective_max_col(ws,rows),500)
    typed=[]
    if ws.title=='formats':
        for pos in ['B2','B3','B4','B5','B6','B7','B8']:
            c=ws[pos];v=c.value
            from datetime import datetime,date,time,timedelta
            kind='numeric' if isinstance(v,(int,float)) else ('date' if isinstance(v,(datetime,date,time)) else 'other')
            typed.append(dict(row=c.row-1,column=c.column-1,kind=kind))
    cases.append(dict(index=i,name=ws.title,last_row=rows-1,last_column=cols-1,typed=typed))
buffer=io.BytesIO()
with zipfile.ZipFile(out) as original,zipfile.ZipFile(buffer,'w') as padded:
    for info in original.infolist(): padded.writestr(info,original.read(info.filename))
    padded.writestr('reference-padding.bin',bytes(5*1024*1024+1),compress_type=zipfile.ZIP_STORED)
raw=buffer.getvalue();assert len(raw)>5*1024*1024
large=read_workbook('large.xlsx',io.BytesIO(raw))
large_cases=[]
for i,sheet in enumerate(large.sheets):
    cells=[]
    for (r,c),cell in sorted(sheet.cells.items()):
        v=cell.value
        from datetime import datetime,date,time,timedelta
        kind=None if v is None else ('numeric' if isinstance(v,(int,float)) else ('date' if isinstance(v,(datetime,date,time)) else ('other' if isinstance(v,timedelta) else 'text')))
        cells.append(dict(row=r-1,column=c-1,kind=kind,text=str(v) if kind=='text' else None,bold=cell.fmt.bold,merged=cell.is_merged,formula=cell.formula))
    large_cases.append(dict(index=i,name=sheet.name,last_row=min(sheet.max_row,10000)-1,last_column=min(sheet.max_col,500)-1,cells=cells))
# Exact 5 MiB remains on the single-pass path.
def padded_bytes(size):
    buf=io.BytesIO()
    with zipfile.ZipFile(out) as original,zipfile.ZipFile(buf,'w') as z:
        for info in original.infolist(): z.writestr(info,original.read(info.filename))
        z.writestr('reference-padding.bin',bytes(size),compress_type=zipfile.ZIP_STORED)
    return buf.getvalue()
base=padded_bytes(0); exact=padded_bytes(5*1024*1024-len(base));assert len(exact)==5*1024*1024
boundary=read_workbook('boundary.xlsx',io.BytesIO(exact))
idx=next(i for i,s in enumerate(boundary.sheets) if s.name=='bold_blank')
cell=boundary.sheets[idx].cells.get((1,2))
print(json.dumps(dict(cases=cases,large_cases=large_cases,boundary=dict(index=idx,bold_blank_present=cell is not None,bold=cell.fmt.bold if cell else None)),ensure_ascii=False))
'''
path=root/'xlsx-window-cases.xlsx'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code,str(path)],text=True,capture_output=True,check=True)
(root/'harunobu-xlsx-window-reference.json').write_text(json.dumps({'harunobu_commit':commit,**json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
