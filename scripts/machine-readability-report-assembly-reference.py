#!/usr/bin/env python3
"""Capture file-level JSON assembly from pinned Harunobu."""
import json,subprocess,sys
from pathlib import Path
repo=Path(sys.argv[1]).resolve(); commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json
from harunobu.core.models import AnalysisResult,FileMeta,SheetResult,SheetMeta,SheetProperty,OtherCell,TableResult,TableLayout,CellRange,ColumnHeader,ColumnSchema,MRResult,CheckResult,Violation
from harunobu.core.severity import Severity
from harunobu.output.json_writer import to_dict,to_bulk_dict
from harunobu.rules.bulk_base import BulkAnalysisResult
from harunobu.output.csv_writer import to_csv_string,to_bulk_csv_string
c1=CheckResult(rule_id='L1-01',passed=True,confidence=1,severity=Severity.FATAL,message='ok')
c2=CheckResult(rule_id='L2-03',passed=False,confidence=.95,severity=Severity.FATAL,message='重複',violations=[Violation(sheet='表',cell_range='A2',description='重複',severity='error')])
c3=CheckResult(rule_id='L3-09',passed=False,confidence=.7,severity=Severity.MAJOR,message='引用: \"a,b\"\n次行',violations=[Violation(sheet='表',cell_range=f'B{i+1}',description=f'詳細,\"{i}\"',severity='warning' if i%2 else 'info') for i in range(25)])
table=TableResult(range=CellRange(start_row=2,start_col=1,end_row=4,end_col=2),confidence=.9,layout=TableLayout(header_rows=[2]),column_headers=[ColumnHeader(col_index=1,header_rows=[2],label='項目',is_merged=False)],columns=[ColumnSchema(col_index=1,inferred_type='text',has_header=True)],mr_result=MRResult(level1={'L1-01':c1},level2={'L2-03':c2},level3={'L3-09':c3}))
sheet=SheetResult(sheet_meta=SheetMeta(name='表',used_range='A1:B4',hidden=False),sheet_property=SheetProperty(sheet_name='表',max_row=4,max_col=2,hidden=False),others=[OtherCell(row=1,col=1,value='タイトル',region_type='title')],tables=[table])
empty=SheetResult(sheet_meta=SheetMeta(name='空',used_range='',hidden=True))
analysis=AnalysisResult(file_meta=FileMeta(name='例.xlsx',size=123,format='xlsx',sheet_count=2),sheets=[sheet,empty])
empty_analysis=AnalysisResult(file_meta=FileMeta(name='空.csv',size=0,format='csv',sheet_count=0),sheets=[])
bulk_check=CheckResult(rule_id='L3-07',passed=False,confidence=.8,severity=Severity.CRITICAL,message='相違',violations=[Violation(sheet='(bulk)',cell_range='',description='相違',severity='warning')])
bulk_result=BulkAnalysisResult(files=[analysis,empty_analysis],bulk_checks={'L3-07':bulk_check})
print(json.dumps(dict(file=to_dict(analysis),bulk=to_bulk_dict(bulk_result),empty=to_bulk_dict(BulkAnalysisResult()),file_csv=to_csv_string(analysis),bulk_csv=to_bulk_csv_string(bulk_result),empty_csv=to_bulk_csv_string(BulkAnalysisResult())),ensure_ascii=False))
'''
data=json.loads(subprocess.check_output([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],cwd=repo,text=True))
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-report-assembly-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'expected':data},ensure_ascii=False,separators=(',',':'))+'\n')
