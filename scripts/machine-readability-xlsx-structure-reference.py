#!/usr/bin/env python3
"""Capture complete detected-table XLSX structure rules from pinned Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability'
code=r'''
import io,json,sys
from pathlib import Path
from harunobu.core.reader import read_workbook
from harunobu.core.analyzer import Analyzer
out=[]
for file in ['structure.xlsx','header-merge.xlsx','xlsx-artifact-errors.xlsx','xlsx-artifact-multisheet.xlsx']:
    wb=read_workbook(file,io.BytesIO((Path(sys.argv[1])/file).read_bytes()));result=Analyzer('thorough').analyze(wb)
    sheets=[]
    for raw_sheet,sheet in zip(wb.sheets,result.sheets):
        tables=[]
        for t in sheet.tables:
            checks=[]
            for group,ids in [(t.mr_result.level1,['L1-01','L1-02','L1-03','L1-04','L1-12','L1-13']),(t.mr_result.level2,['L2-06'])]:
                for id in ids:
                    r=group[id];checks.append(dict(id=id,passed=r.passed,confidence=r.confidence,severity=r.severity.value,findings=[[v.cell_range,v.severity] for v in r.violations]))
            r=t.range;tables.append(dict(range=[r.start_row-1,r.start_col-1,r.end_row-1,r.end_col-1],headers=[r-1 for r in t.layout.header_rows],confidence=t.confidence,checks=checks))
        sheets.append(dict(name=raw_sheet.name,tables=tables))
    out.append(dict(file=file,sheets=sheets))
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code,str(root)],text=True,stdout=subprocess.PIPE,check=True,env=env)
(root/'harunobu-xlsx-structure-reference.json').write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
