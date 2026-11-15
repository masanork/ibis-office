#!/usr/bin/env python3
"""Capture AI-disabled CSV L2-04 from pinned Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.models import Cell,CellPosition,ColumnSchema,Sheet,WorkBook,TableContext,TableRegion,TableLayout,CellRange
from harunobu.rules.level2.L2_04_choice_standardization import ChoiceStandardizationRule, _find_outliers, _is_variant_of, _normalize
rng=random.Random(204);cases=[]
for i in range(100):
    cols=rng.randint(1,4);rows=rng.randint(15,45);cells=[]
    schemas=[] if i%4==0 else [[c,rng.choice(['numeric','mixed','text','date','empty'])] for c in range(cols) if rng.random()<.8]
    values=['東京都','東京都','東京都','東京都','東京都','東京都','東京都','東京都','東京都（注）','東京','大阪府','大阪府','大阪府','', ' ']+[f'値{j}' for j in range(i%28)]
    for r in range(rows):
        for c in range(cols):
            if rng.random()<.95: cells.append([r,c,rng.choice(values)])
    mapping={(r+1,c+1):Cell(pos=CellPosition(row=r+1,col=c+1),value=v) for r,c,v in cells}
    sheet=Sheet(name='CSV',cells=mapping,max_row=rows,max_col=cols);workbook=WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
    columns=[ColumnSchema(col_index=c+1,inferred_type=kind) for c,kind in schemas]
    region=TableRegion(range=CellRange(start_row=1,start_col=1,end_row=rows,end_col=cols),layout=TableLayout(columns=columns,body_start_row=1,body_end_row=rows),confidence=1)
    result=ChoiceStandardizationRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region))
    alternatives=[]
    for violation in result.violations:
        item=next(cell for cell in mapping.values() if str(cell.pos)==violation.cell_range)
        candidate=str(item.value).strip(); col_values={}
        for row in range(1,rows+1):
            cell=sheet.get_cell(row,item.pos.col)
            if cell and cell.value is not None and str(cell.value).strip(): col_values.setdefault(str(cell.value).strip(),[]).append(row)
        outliers=_find_outliers(col_values,sum(len(v) for v in col_values.values()))
        matches=[v for v in col_values if v not in outliers and _is_variant_of(candidate,{v:_normalize(v)})[0]]
        count=len(col_values[candidate])
        alternatives.append([violation.cell_range,[f"表記揺れの可能性: '{candidate}' は '{value}' と類似（出現{count}回）" for value in matches]])
    cases.append(dict(rows=rows,cols=cols,schemas=schemas,cells=cells,passed=result.passed,confidence=result.confidence,message=result.message,descriptions=alternatives,findings=[[v.cell_range,v.severity] for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-choice-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
