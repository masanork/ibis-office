#!/usr/bin/env python3
"""Capture openpyxl number format classification from pinned Harunobu env."""
import json
from pathlib import Path
import subprocess
import sys
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from openpyxl.styles.numbers import is_date_format,is_timedelta_format
rng=random.Random(909489)
formats=['General','0','0.00','#,##0.00','0%','yyyy-mm-dd','m/d/yy','hh:mm:ss','[h]:mm:ss','[H]:MM','[Red]0','"days"0','\\d0','_d0','[hh]','[H]','[hhh]','d;0','0;yyyy','[red]d','[h"x"]','"d','"a\nd"','[x\ny]d']
atoms=['y','m','d','h','s','Y','M','D','H','S','0','#','[',']','"','\\','_',':',';','x',' ','\n']
for _ in range(1200): formats.append(''.join(rng.choice(atoms) for _ in range(rng.randrange(0,30))))
print(json.dumps([dict(format=f,kind=('duration' if is_timedelta_format(f) else 'date') if is_date_format(f) else 'numeric') for f in formats],ensure_ascii=False))
'''
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-xlsx-formats-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
