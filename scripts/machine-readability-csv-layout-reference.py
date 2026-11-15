#!/usr/bin/env python3
"""Capture CSV bytes through layout detection from pinned Harunobu."""
import json
from pathlib import Path
import subprocess
import sys
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json
from harunobu.core.reader import _read_csv_data
from harunobu.core.layout.detector import LayoutDetector
from harunobu.core.models import Config
inputs=[('地域,年度,人数\n東京,2024,5\n大阪,2025,10\n','utf-8',','),('品目\t価格\nりんご\t100\nみかん\t200\n','cp932','\t'),('名前,値\n太郎,001\n花子,002\n','utf-8-sig',','),('項目,値\n私用\uf8f0,123\n標準,456\n','cp932',',')]
cases=[]
for text,encoding,delimiter in inputs:
    raw=text.encode(encoding); workbook=_read_csv_data('fixture.csv',raw,delimiter); sheet=workbook.sheets[0]
    result=LayoutDetector(Config(mode='standard')).detect_with_others(sheet)
    assert len(result.tables)<=1
    cases.append(dict(bytes=raw.hex(),delimiter=delimiter,encoding=workbook.encoding,tables=[[t.range.start_row-1,t.range.start_col-1,t.range.end_row-1,t.range.end_col-1,t.confidence,[r-1 for r in t.layout.header_rows]] for t in result.tables],others=[[o.row-1,o.col-1,o.region_type] for o in result.others]))
print(json.dumps(cases,ensure_ascii=False))
'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-csv-layout-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
