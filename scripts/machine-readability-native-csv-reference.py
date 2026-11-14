#!/usr/bin/env python3
"""Record partial CSV Level 1 outcomes from pinned Harunobu."""
import json,os
from pathlib import Path
import subprocess,sys
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json
from harunobu.core.reader import _read_csv_data
from harunobu.core.analyzer import Analyzer
inputs=[('clean','地域,年度,人数\n東京,2024,5\n大阪,2025,10\n','utf-8',','),('bom','地域,年度,人数\n東京,2024,5\n大阪,2025,10\n','utf-8-sig',','),('cp932','品目,区分,数量\n①,A,10\n大阪,B,20\n','cp932',','),('enum','地域,内容,数量\n東京,A、B、C,10\n大阪,通常,20\n','utf-8',','),('multi','地域,内容,数量\n東京,"一行\n二行",10\n大阪,通常,20\n','utf-8',','),('title','調査の表,,\n地域,年度,人数\n東京,2024,5\n大阪,2025,10\n','utf-8',','),('tsv','地域\t内容\t数量\n東京\tA・B・C\t10\n大阪\t通常\t20\n','utf-8','\t'),('fullwidth','項目,値,数\nＡ,通常,1\nＢ,通常,2\n','utf-8',',')]
inputs.extend([('address','住所,内容,数量\n東京都\u3000千代田区,通常,1\n大阪府\u3000大阪市,空白\u3000あり,2\n','utf-8',','),('whitespace','地域,内容,数量\n東京, 先頭,1\n大阪,通常  空白,2\n','utf-8',',')])
inputs.append(('wide-years','地域,2021,2022,2023\n東京,1,2,3\n大阪,4,5,6\n','utf-8',','))
inputs.append(('choice-variant','地域,数\n'+'東京都,1\n'*20+'東京都（注）,1\n','utf-8',','))
inputs.append(('other-details','回答,数\n'+'選択,1\n'*18+'その他（詳細）,1\nその他,1\n選択,1\n','utf-8',','))
inputs.append(('numeric-mixed','地域,金額,人数\n東京,100円,5\n大阪,200,6\n京都,300,10\n','utf-8',','))
inputs.append(('abbreviation','地域,分類,人数\n東京,A,5\n同上,B,6\n大阪,C,10\n','utf-8',','))
inputs.append(('missing-header','地域,,人数\n東京,関東,5\n大阪,近畿,10\n','utf-8',','))
inputs.append(('duplicate-headers','地域,地域,人数\n東京,大阪,5\n大阪,京都,10\n','utf-8',','))
ids=['L1-01','L1-02','L1-03','L1-04','L1-05','L1-06','L1-07','L1-08','L1-09','L1-14','L1-15']
out=[]
for name,text,enc,delimiter in inputs:
    raw=text.encode(enc);wb=_read_csv_data('fixture.csv',raw,delimiter);result=Analyzer('thorough').analyze(wb);tables=result.sheets[0].tables
    assert len(tables)==1
    table=tables[0];checks=[]
    for id in ids:
        r=table.mr_result.level1[id]
        checks.append(dict(id=id,passed=r.passed,confidence=r.confidence,severity=r.severity.value,findings=[[v.cell_range,v.severity] for v in r.violations]))
    level2_checks=[]
    for id in ['L2-01','L2-02','L2-03','L2-04','L2-05']:
        l2=table.mr_result.level2[id]
        level2_checks.append(dict(id=id,passed=l2.passed,confidence=l2.confidence,severity=l2.severity.value,findings=[[v.cell_range,v.severity] for v in l2.violations]))
    level3_checks=[]
    for id in ['L3-01','L3-02','L3-03','L3-07','L3-08','L3-09']:
        r=table.mr_result.level3[id]
        level3_checks.append(dict(id=id,passed=r.passed,confidence=r.confidence,severity=r.severity.value,findings=[[v.cell_range,v.severity] for v in r.violations]))
    r=table.range
    out.append(dict(level3_checks=level3_checks,level2_checks=level2_checks,name=name,bytes=raw.hex(),delimiter=delimiter,encoding=wb.encoding,range=[r.start_row-1,r.start_col-1,r.end_row-1,r.end_col-1],headers=[r-1 for r in table.layout.header_rows],confidence=table.confidence,checks=checks))
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','GOOGLE_APPLICATION_CREDENTIALS','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-native-csv-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
