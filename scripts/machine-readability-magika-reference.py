#!/usr/bin/env python3
"""Capture file-type labels from the pinned Harunobu reference environment."""
import json,subprocess,sys
from pathlib import Path
repo=Path(sys.argv[1]).resolve();commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability'
cases=[]
for name,limit in [('harunobu-csv-report-e2e-reference.json',999),('harunobu-xlsx-generated-reference.json',15)]:
    for i,case in enumerate(json.loads((root/name).read_text())['cases'][:limit]): cases.append(dict(fixture=name,index=i,bytes=case['bytes']))
cases += [dict(fixture=None,index=i,bytes=x.hex()) for i,x in enumerate([b'',b'%PDF-1.7\nhello',b'\x89PNG\r\n\x1a\n'+bytes(100),bytes(40),b'PK\x03\x04'+bytes(40)])]
code='''import json,sys\nfrom magika import Magika\nm=Magika()\nprint(json.dumps([m.identify_bytes(bytes.fromhex(x)).output.label for x in json.load(sys.stdin)]))'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-c',code],input=json.dumps([v['bytes'] for v in cases]),text=True,capture_output=True,check=True,cwd=repo)
for case,label in zip(cases,json.loads(run.stdout)):
    case['label']=label
    if case['fixture'] is not None: del case['bytes']
(root/'harunobu-magika-reference.json').write_text(json.dumps({'harunobu_commit':commit,'cases':cases},ensure_ascii=False,separators=(',',':'))+'\n')
