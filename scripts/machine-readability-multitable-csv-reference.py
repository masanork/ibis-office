#!/usr/bin/env python3
"""Capture end-to-end multi-table CSV analysis from pinned Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import csv,io,json,random
from harunobu.core.reader import _read_csv_data
from harunobu.core.analyzer import Analyzer
inputs=[('two','地域,値\n東京,1\n大阪,2\n\n\n\n地域,値\n京都,3\n青森,4\n'),('different','地域,値,数\n東京, A,1\n大阪,B,2\n\n\n\n名前,数,率\n名古屋,3,10%\n札幌,4,20%\n'),('side','地域,値,,,,項目,数\n東京,1,,,,A,5\n大阪,2,,,,B,6\n'),('unequal','地域,値\n東京,1\n大阪,2\n京都,3\n\n\n\n項目,数\n品目,5\nその他,6\n')]
ids1=['L1-01','L1-02','L1-03','L1-04','L1-05','L1-06','L1-07','L1-08','L1-09','L1-14','L1-15']
ids2=['L2-01','L2-02','L2-03','L2-04','L2-05'];ids3=['L3-01','L3-02','L3-03','L3-04','L3-05','L3-06','L3-07','L3-08','L3-09']
inputs.append(('inherit','地域,年度,値\n東京,2024,1\n大阪,2025,2\n\n\n京都,2026,3\n青森,2027,4\n'))
rng=random.Random(20261114)
for i in range(160):
    stream=io.StringIO(newline='');writer=csv.writer(stream,lineterminator='\n')
    for block in range(rng.randint(1,3)):
        if block:
            for _ in range(rng.randint(2,4)): writer.writerow([])
        cols=rng.randint(2,5);writer.writerow([rng.choice(['地域','値','人数','分類','年度','金額','項目']) for _ in range(cols)])
        for _ in range(rng.randint(2,7)):
            writer.writerow([rng.choice(['東京','大阪','北海道','1','2','100円','', '同上','通常','３','品目']) for _ in range(cols)])
    inputs.append((f'generated-{i}',stream.getvalue()))
out=[]
for name,text in inputs:
    raw=text.encode(); result=Analyzer('thorough').analyze(_read_csv_data('fixture.csv',raw,','));tables=[]
    for t in result.sheets[0].tables:
        checks=[]
        for level,ids in [(t.mr_result.level1,ids1),(t.mr_result.level2,ids2),(t.mr_result.level3,ids3)]:
            for id in ids:
                r=level[id];checks.append(dict(id=id,passed=r.passed,confidence=r.confidence,severity=r.severity.value,findings=[[v.cell_range,v.severity] for v in r.violations]))
        r=t.range;tables.append(dict(range=[r.start_row-1,r.start_col-1,r.end_row-1,r.end_col-1],headers=[r-1 for r in t.layout.header_rows],confidence=t.confidence,checks=checks))
    out.append(dict(name=name,bytes=raw.hex(),tables=tables))
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-multitable-csv-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
