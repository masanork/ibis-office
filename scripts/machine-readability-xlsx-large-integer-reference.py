#!/usr/bin/env python3
"""Capture arbitrary-precision saved integers from pinned Harunobu."""
import io,json,os,subprocess,sys
from pathlib import Path
repo=Path(sys.argv[1]).resolve();commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import io,json
from openpyxl import Workbook
from harunobu.core.reader import read_workbook
from harunobu.core.analyzer import Analyzer
from harunobu.output.json_writer import to_dict
book=Workbook();sheet=book.active;sheet.title='値';sheet['A1']=12345
stream=io.BytesIO();book.save(stream)
import zipfile
modified=io.BytesIO()
with zipfile.ZipFile(io.BytesIO(stream.getvalue())) as source, zipfile.ZipFile(modified,'w') as target:
    for entry in source.infolist():
        content=source.read(entry.filename)
        if entry.filename=='xl/worksheets/sheet1.xml': content=content.replace(b'<v>12345</v>',b'<v>123456789012345678901234567890123456789</v>')
        target.writestr(entry,content)
raw=modified.getvalue();wb=read_workbook('integer.xlsx',io.BytesIO(raw))
out=dict(bytes=raw.hex(),reports={mode:to_dict(Analyzer(mode).analyze(wb)) for mode in ['lite','standard','thorough']})
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-xlsx-large-integer-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,**json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
