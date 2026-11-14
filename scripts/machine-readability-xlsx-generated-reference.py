#!/usr/bin/env python3
"""Generate synthetic typed, formatted XLSX parity cases from Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import datetime,io,json,random
from openpyxl import Workbook
from openpyxl.styles import Border,Font,PatternFill,Side
from harunobu.core.reader import read_workbook
from harunobu.core.analyzer import Analyzer
from harunobu.core.scorer import LevelScorer
rng=random.Random(110621);out=[]
ids1=['L1-01','L1-02','L1-03','L1-04','L1-05','L1-06','L1-07','L1-08','L1-09','L1-10','L1-11','L1-12','L1-13']
ids2=['L2-01','L2-02','L2-03','L2-04','L2-05','L2-06'];ids3=['L3-01','L3-02','L3-03','L3-04','L3-05','L3-06','L3-07','L3-08','L3-09']
headers=['地域','値','人数','分類','年度','金額','項目','期間'];values=['東京','大阪','北海道',1,2,3.5,True,False,'100円',None,'同上','通常','①','品目',datetime.datetime(2024,1,2)]
for i in range(100):
    book=Workbook();ws=book.active;ws.title='Data';cols=rng.randint(2,5);rows=rng.randint(3,9)
    ws.append([rng.choice(headers) for _ in range(cols)])
    for r in range(1,rows):
        ws.append([rng.choice(values) for _ in range(cols)])
        for c in range(1,cols+1):
            cell=ws.cell(r+1,c)
            if rng.random()<.12: cell.font=Font(bold=True,italic=rng.random()<.5,color=rng.choice(['FF0000','000000','FFFFFF']))
            if rng.random()<.08: cell.fill=PatternFill('solid',fgColor=rng.choice(['00FF00','FFFFFF','000000']))
            if rng.random()<.05: cell.border=Border(left=Side(style='thin'),right=Side(),top=Side(),bottom=Side())
    if i%4==0 and cols>=3: ws.merge_cells(start_row=1,start_column=1,end_row=1,end_column=2)
    if i%7==0: ws.row_dimensions[rows].hidden=True
    buf=io.BytesIO();book.save(buf);raw=buf.getvalue();wb=read_workbook('fixture.xlsx',io.BytesIO(raw));result=Analyzer('thorough').analyze(wb);tables=[]
    for t in result.sheets[0].tables:
        checks=[]
        for level,ids in [(t.mr_result.level1,ids1),(t.mr_result.level2,ids2),(t.mr_result.level3,ids3)]:
            for id in ids:
                r=level[id];checks.append(dict(id=id,passed=r.passed,confidence=r.confidence,severity=r.severity.value,findings=[[v.cell_range,v.severity] for v in r.violations]))
        r=t.range;tables.append(dict(range=[r.start_row-1,r.start_col-1,r.end_row-1,r.end_col-1],headers=[v-1 for v in t.layout.header_rows],confidence=t.confidence,checks=checks))
    scores=LevelScorer().score_analysis(result)
    out.append(dict(name=f'generated-{i}',bytes=raw.hex(),tables=tables,scores=[scores.per_level[n].model_dump(mode='json') for n in (1,2,3)]))
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,stdout=subprocess.PIPE,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-xlsx-generated-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
