#!/usr/bin/env python3
"""Capture ISO date cells from pinned Harunobu."""
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
from datetime import date, datetime, time
book=Workbook(iso_dates=True);sheet=book.active;sheet.title='日付';sheet['A1']=date(2024,2,29);sheet['A3']=datetime(2025,1,2,3,4,5);sheet['A5']=time(12,34,56);sheet['C1']='日付';sheet['D1']='値';sheet['C2']=date(2023,5,1);sheet['D2']=10;sheet['C3']=datetime(2024,2,29,1,2,3,456000);sheet['D3']=20
stream=io.BytesIO();book.save(stream);raw=stream.getvalue();wb=read_workbook('iso-date.xlsx',io.BytesIO(raw))
out=dict(bytes=raw.hex(),reports={mode:to_dict(Analyzer(mode).analyze(wb)) for mode in ['lite','standard','thorough']})
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-xlsx-iso-date-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,**json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
