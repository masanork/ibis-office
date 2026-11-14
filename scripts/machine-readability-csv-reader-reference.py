#!/usr/bin/env python3
"""Capture CSV reader results from a pinned Harunobu checkout."""
import json
from pathlib import Path
import subprocess
import sys
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import csv,io,json,random
from harunobu.core.reader import _parse_csv_text
rng=random.Random(920171)
texts=['','\n','\r','\r\r\n','a\rb','a\r\nb','""','"a""','""x','"""x"','a,','a,\n','"a\rb"','a\r\rb','1,02,=1+1,2024-01-01','a\n\nb\n','"a\n b",x','  "x",y','"a"x,y']
atoms=['a','1',' ','日','"',',','\t','\n','\r','\0']
for _ in range(900): texts.append(''.join(rng.choice(atoms) for _ in range(rng.randrange(0,45))))
cases=[]
for delim in [',','\t']:
    for text in texts:
        try:
            sheet=_parse_csv_text(text,'fixture',delim)
            records=list(csv.reader(io.StringIO(text),delimiter=delim))
            rows=[[v if v!='' else None for v in record] for record in records]
            assert sheet.max_row==len(rows)
            assert sheet.max_col==max([len(row) for row in rows],default=0)
            assert all(sheet.cells[(r+1,c+1)].value==v for r,row in enumerate(rows) for c,v in enumerate(row))
            cases.append(dict(text=text,delimiter=delim,records=rows,max_columns=sheet.max_col,error=None))
        except csv.Error:
            cases.append(dict(text=text,delimiter=delim,records=None,max_columns=None,error='unquoted_carriage_return'))
print(json.dumps(cases,ensure_ascii=False))
'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-csv-reader-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
