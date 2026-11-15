#!/usr/bin/env python3
"""Capture synthetic XLSX cached values and formula references from Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability'
code=r'''
import io,json,sys
from pathlib import Path
from harunobu.core.reader import read_workbook
out=[]
for name in ['missing','numeric','boolean','text','formula_text','error']:
    file=f'xlsx-cache-{name}.xlsx'
    sheet=read_workbook(file,io.BytesIO((Path(sys.argv[1])/file).read_bytes())).sheets[0]
    c=sheet.cells.get((5,3));v=c.value if c else None
    out.append(dict(file=file,value=v,kind='none' if v is None else 'bool' if isinstance(v,bool) else 'number' if isinstance(v,(int,float)) else 'text'))
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code,str(root)],text=True,capture_output=True,check=True,env=env)
(root/'harunobu-xlsx-saved-values-reference.json').write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
