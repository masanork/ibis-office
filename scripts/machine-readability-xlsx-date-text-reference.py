#!/usr/bin/env python3
"""Capture openpyxl Excel date rendering from the pinned reference environment."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from openpyxl.utils.datetime import from_excel,WINDOWS_EPOCH,MAC_EPOCH
rng=random.Random(1904)
values=[0,0.5,0.999999999,1,59,60,61,45293,45293.5,0.0000001,2_900_000]
values += [rng.uniform(0,2_900_000) for _ in range(600)]
print(json.dumps([dict(value=repr(v),date1904=flag,expected=str(from_excel(v,epoch=MAC_EPOCH if flag else WINDOWS_EPOCH))) for flag in [False,True] for v in values]))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-xlsx-date-text-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},separators=(',',':'))+'\n')
