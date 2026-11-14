#!/usr/bin/env python3
"""Capture Python number rendering used by the pinned reference reader."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random,math
rng=random.Random(30421)
values=[0.0,-0.0,1.0,-1.0,1e-4,1e-5,1e15,1e16,1e20,1e-10,1.2345678901234567,2.2250738585072014e-308,5e-324,1.7976931348623157e308]
for _ in range(1000):
    value=math.ldexp(rng.uniform(-1,1),rng.randrange(-1000,1000))
    if math.isfinite(value):values.append(value)
out=[dict(kind='float',raw=repr(v),expected=str(v)) for v in values]
out.extend(dict(kind='integer',raw=raw,expected=str(int(raw))) for raw in ['0','-0','+000','00012','-0012','9007199254740993','-9007199254740993123456789'])
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-xlsx-number-text-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
