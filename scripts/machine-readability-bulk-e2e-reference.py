#!/usr/bin/env python3
"""Capture mixed-format bulk report output from pinned Harunobu."""
import io,json,os,subprocess,sys
from pathlib import Path
repo=Path(sys.argv[1]).resolve();commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import io,json
from openpyxl import Workbook
from harunobu.core.reader import read_workbook
from harunobu.core.analyzer import Analyzer
from harunobu.output.json_writer import to_bulk_dict
w=Workbook();s=w.active;s.title='Data';s.append(['地域','値']);s.append(['東京',1]);s.append(['大阪',2]);b=io.BytesIO();w.save(b)
inputs=[('a.csv','地域,値\n東京,1\n大阪,2\n'.encode()),('b.tsv','地域\t値\n東京\t3\n大阪\t4\n'.encode()),('c.xlsx',b.getvalue()),('d.csv','名前,値\n青森,2\n秋田,3\n'.encode())]
workbooks=[read_workbook(name,io.BytesIO(raw)) for name,raw in inputs]
cases=[]
for indices in [[0,1],[0,2],[0,1,2,3]]:
 for mode in ['lite','standard','thorough']:
    result=Analyzer(mode).analyze_bulk([workbooks[i] for i in indices])
    cases.append(dict(indices=indices,mode=mode,report=to_bulk_dict(result)))
print(json.dumps(dict(inputs=[dict(name=name,bytes=raw.hex()) for name,raw in inputs],cases=cases),ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-bulk-e2e-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,**json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
