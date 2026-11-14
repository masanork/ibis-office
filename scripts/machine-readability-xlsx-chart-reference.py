#!/usr/bin/env python3
"""Capture chart-sheet and worksheet ordering from pinned Harunobu."""
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
book=Workbook();first=book.active;first.title='最初';first.append(['地域','値']);first.append(['東京',1]);first.append(['大阪',2])
from openpyxl.chart import BarChart, Reference
chart=BarChart();chart.add_data(Reference(first,min_col=2,min_row=1,max_row=3),titles_from_data=True)
chartsheet=book.create_chartsheet('グラフ',0);chartsheet.add_chart(chart)
last=book.create_sheet('最後');last.append(['分類','金額']);last.append(['A',100]);last.append(['B',200])
stream=io.BytesIO();book.save(stream);raw=stream.getvalue();wb=read_workbook('chart.xlsx',io.BytesIO(raw))
out=dict(bytes=raw.hex(),reports={mode:to_dict(Analyzer(mode).analyze(wb)) for mode in ['lite','standard','thorough']})
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-xlsx-chart-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,**json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
