#!/usr/bin/env python3
"""Capture integer/float distinctions from synthetic saved XLSX numbers."""
import json,os,re,subprocess,sys,zipfile
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability'
with zipfile.ZipFile(root/'structure.xlsx') as src,zipfile.ZipFile(root/'xlsx-numeric-lexemes.xlsx','w') as dst:
    for info in src.infolist():
        data=src.read(info.filename)
        if info.filename=='xl/worksheets/sheet1.xml':
            text=data.decode('utf8')
            for ref,value,formula in [('A2','9007199254740993',''),('B3','1.0',''),('C4','1e+20',''),('C5','1.0','<f>SUM(C2:C4)</f>')]:
                text,count=re.subn(r'<c\b[^>]*\br="'+ref+r'"[^>]*>.*?</c>',f'<c r="{ref}" t="n">{formula}<v>{value}</v></c>',text);assert count==1
            data=text.encode('utf8')
        dst.writestr(info,data)
code=r'''
import io,json,sys
from pathlib import Path
from harunobu.core.reader import read_workbook
sheet=read_workbook('x.xlsx',io.BytesIO((Path(sys.argv[1])/'xlsx-numeric-lexemes.xlsx').read_bytes())).sheets[0]
out=[]
for row,col,raw in [(2,1,'9007199254740993'),(3,2,'1.0'),(4,3,'1e+20'),(5,3,'1.0')]:
    value=sheet.get_cell(row,col).value
    out.append(dict(row=row-1,col=col-1,raw=raw,kind='integer' if type(value) is int else 'float',reference=str(value)))
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code,str(root)],text=True,capture_output=True,check=True,env=env)
(root/'harunobu-xlsx-numeric-lexeme-reference.json').write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
