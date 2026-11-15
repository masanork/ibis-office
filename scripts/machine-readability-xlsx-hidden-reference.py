#!/usr/bin/env python3
"""Capture hidden and visible worksheet ordering from pinned Harunobu."""
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
book=Workbook();hidden=book.active;hidden.title='非表示';hidden.append(['地域','値']);hidden.append(['東京',1]);hidden.append(['大阪',2]);hidden.sheet_state='hidden'
visible=book.create_sheet('表示');visible.append(['分類','金額']);visible.append(['A',100]);visible.append(['B',200]);book.active=1
very=book.create_sheet('完全非表示');very.append(['項目','数']);very.append(['例',3]);very.append(['値',4]);very.sheet_state='veryHidden'
stream=io.BytesIO();book.save(stream);raw=stream.getvalue();wb=read_workbook('hidden.xlsx',io.BytesIO(raw))
out=dict(bytes=raw.hex(),reports={mode:to_dict(Analyzer(mode).analyze(wb)) for mode in ['lite','standard','thorough']})
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-xlsx-hidden-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,**json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
