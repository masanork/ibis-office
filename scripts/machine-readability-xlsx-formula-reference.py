#!/usr/bin/env python3
"""Generate synthetic cached-formula XLSX fixtures and Harunobu outcomes."""
import json
from pathlib import Path
import re
import subprocess
import sys
import zipfile
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability'
variants={'missing':'<c r="C5"><f>SUM(C2:C4)</f><v /></c>', 'numeric':'<c r="C5" t="n"><f>SUM(C2:C4)</f><v>999</v></c>', 'boolean':'<c r="C5" t="b"><f>SUM(C2:C4)</f><v>1</v></c>', 'text':'<c r="C5" t="str"><f>SUM(C2:C4)</f><v>cached text</v></c>', 'formula_text':'<c r="C5" t="str"><f>SUM(C2:C4)</f><v>=cached</v></c>', 'error':'<c r="C5" t="e"><f>SUM(C2:C4)</f><v>#DIV/0!</v></c>'}
for name,replacement in variants.items():
    with zipfile.ZipFile(root/'structure.xlsx') as source, zipfile.ZipFile(root/f'xlsx-cache-{name}.xlsx','w') as out:
        for info in source.infolist():
            data=source.read(info.filename)
            if info.filename=='xl/worksheets/sheet1.xml':
                text=data.decode('utf-8')
                text,count=re.subn(r'<c\b[^>]*\br="C5"[^>]*>.*?</c>',replacement,text)
                assert count==1
                data=text.encode('utf-8')
            out.writestr(info,data)
code=r'''
import io,json,sys
from pathlib import Path
from harunobu.core.reader import read_workbook
out=[]
for name in ['missing','numeric','boolean','text','formula_text','error']:
    file=f'xlsx-cache-{name}.xlsx'
    sheet=read_workbook(file,io.BytesIO((Path(sys.argv[1])/file).read_bytes())).sheets[0]
    c=sheet.cells.get((5,3))
    if c is None: expected=None
    else:
        v=c.value
        expected=dict(row=4,column=2,kind=None if v is None else ('numeric' if isinstance(v,(int,float)) else 'text'),text=None if v is None or isinstance(v,(int,float)) else str(v),bold=c.fmt.bold,merged=c.is_merged,formula=c.formula)
    out.append(dict(file=file,expected=expected))
print(json.dumps(out,ensure_ascii=False))
'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code,str(root)],text=True,capture_output=True,check=True)
(root/'harunobu-xlsx-formula-reference.json').write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
