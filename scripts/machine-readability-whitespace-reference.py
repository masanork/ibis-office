#!/usr/bin/env python3
"""Record L1-06 cell-level checks from pinned Harunobu."""
import json,os
from pathlib import Path
import subprocess,sys
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.rules.level1.L1_06_whitespace_formatting import WhitespaceFormattingRule
rng=random.Random(445113); rule=WhitespaceFormattingRule()
values=['',' normal ','\n','行\n','行\r\n','A\nB','十分に長い文章の中に改行\nがある例です','東京　千代田区','ア　項目','a　項目','①　項目','※1　注記','第５　表','　〃','　〃\n','　○　項目','[概況]　1','Ⅰ．内容','1．　見出し','　　1.項目','(B)/(A)　(%)','a  b','\u001c値\u001c','住所　住所','😀\n😀']
atoms=['文','a','1','　',' ','\t','\n','\r','※','〃','○','（','）','①','\u001c','😀']
for _ in range(600): values.append(''.join(rng.choice(atoms) for _ in range(rng.randrange(0,35))))
print(json.dumps([dict(value=v,address=a,issues=rule._check_cell_value(v,is_address_col=a)) for v in values for a in (False,True)],ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-whitespace-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
