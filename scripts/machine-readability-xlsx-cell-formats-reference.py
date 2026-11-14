#!/usr/bin/env python3
"""Capture normal and large XLSX reader formatting from Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability'
code=r'''
import io,json,sys,zipfile
from pathlib import Path
from harunobu.core.reader import read_workbook
raw=(Path(sys.argv[1])/'xlsx-format-variation.xlsx').read_bytes();out=[]
for padded in [False,True]:
    if padded:
        buf=io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(raw)) as src,zipfile.ZipFile(buf,'w') as dst:
            for info in src.infolist(): dst.writestr(info,src.read(info.filename))
            dst.writestr('reference-padding.bin',bytes(5*1024*1024),compress_type=zipfile.ZIP_STORED)
        data=buf.getvalue()
    else:data=raw
    sheet=read_workbook('fixture.xlsx',io.BytesIO(data)).sheets[0];entries=[]
    for row,col in [(1,1),(2,1),(3,1),(3,2),(4,1)]:
        cell=sheet.get_cell(row,col);fmt=cell.fmt
        bg=fmt.bg_color[-6:].upper() if fmt.bg_color else None
        if bg=='FFFFFF':bg=None
        entries.append(dict(row=row-1,col=col-1,font=fmt.font_color,bg=bg,stub_bg=fmt.bg_color is not None and fmt.bg_color != "FFFFFFFF",bold=fmt.bold,italic=fmt.italic,left=fmt.border_left is not None,right=fmt.border_right is not None))
    out.append(dict(padded=padded,entries=entries))
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code,str(root)],text=True,capture_output=True,check=True,env=env)
(root/'harunobu-xlsx-formats-values-reference.json').write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
