#!/usr/bin/env python3
"""Capture typed CSV/TSV report cases from the pinned reference."""
import json,os,subprocess,sys
from pathlib import Path
repo=Path(sys.argv[1]).resolve(); commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json
from harunobu.core.reader import _read_csv_data
from harunobu.core.analyzer import Analyzer
from harunobu.output.json_writer import to_dict
texts=['地域,数\n東京,1\n大阪,2\n','タイトル\n地域,値\n東京,①\n大阪,３\n', '項目,値\n"複数\n行",1\n値,2\n', '地域,値\n東京,1\n大阪,2,extra\n', '地域,値\n東京,ab"cd\n大阪,2\n', '地域,値\n東京,1\n大阪,"open\n']
out=[]
for delimiter,extension in [(',', 'csv'),('\t','tsv')]:
    for text in texts:
        text=text.replace(',',delimiter)
        for encoding in ['utf-8','utf-8-sig','cp932']:
            raw=text.encode(encoding); name='fixture.'+extension
            result=Analyzer('thorough').analyze(_read_csv_data(name,raw,delimiter))
            out.append(dict(name=name,encoding=encoding,bytes=raw.hex(),report=to_dict(result)))
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-csv-report-e2e-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
