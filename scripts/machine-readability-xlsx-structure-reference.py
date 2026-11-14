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
from openpyxl import Workbook
from openpyxl.chart import BarChart,Reference
from openpyxl.styles import Font,PatternFill
book=Workbook();sheet=book.active;sheet.title='Text'
sheet.append(['住所','内容','数']);sheet.append(['東京都　千代田区',' 先頭と末尾 ',1]);sheet.append(['大阪府 大阪市','①Ａ',2]);sheet.append(['京都府 京都市','A、B、C',3]);book.save(Path(sys.argv[1])/'xlsx-text-checks.xlsx')
book=Workbook();sheet=book.active;sheet.title='Tables'
for row in [['地域','数'],['東京',1],['大阪',2],[],[],[],['場所','値'],['京都',3],['青森',4]]: sheet.append(row)
book.save(Path(sys.argv[1])/'xlsx-multiple-tables.xlsx')
book=Workbook();sheet=book.active;sheet.title='Charts'
for row in [['地域','数'],['東京',1],['大阪',2],['京都',3]]: sheet.append(row)
for anchor in ['B2','M20']:
    chart=BarChart();chart.add_data(Reference(sheet,min_col=2,min_row=1,max_row=4),titles_from_data=True);sheet.add_chart(chart,anchor)
book.save(Path(sys.argv[1])/'xlsx-chart-anchors.xlsx')
book=Workbook();sheet=book.active;sheet.title='Style'
for row in [['項目','値'],['A',1],['B',2],['C',3]]: sheet.append(row)
sheet['A3'].font=Font(color='FF0000',bold=True,italic=True);sheet['A3'].fill=PatternFill('solid',fgColor='00FF00')
sheet['B3'].font=Font(color='0000FF');sheet['B3'].fill=PatternFill('solid',fgColor='FFFFFF')
book.save(Path(sys.argv[1])/'xlsx-format-variation.xlsx')
out=[]
for file in ['structure.xlsx','header-merge.xlsx','xlsx-artifact-errors.xlsx','xlsx-artifact-multisheet.xlsx','xlsx-text-checks.xlsx','xlsx-multiple-tables.xlsx','xlsx-chart-anchors.xlsx','xlsx-format-variation.xlsx']:
    wb=read_workbook(file,io.BytesIO((Path(sys.argv[1])/file).read_bytes()));result=Analyzer('thorough').analyze(wb)
    sheets=[]
    for raw_sheet,sheet in zip(wb.sheets,result.sheets):
        tables=[]
        for t in sheet.tables:
            checks=[]
            for group,ids in [(t.mr_result.level1,['L1-01','L1-02','L1-03','L1-04','L1-05','L1-06','L1-07','L1-08','L1-09','L1-10','L1-11','L1-12','L1-13']),(t.mr_result.level2,['L2-06'])]:
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
