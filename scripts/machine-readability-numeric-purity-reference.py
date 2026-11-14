#!/usr/bin/env python3
"""Capture AI-disabled CSV L2-01 from pinned Harunobu."""
import json,os,subprocess,sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
code=r'''
import json,random
from harunobu.core.models import Cell,CellPosition,ColumnSchema,Sheet,WorkBook,TableContext,TableRegion,TableLayout,CellRange
from harunobu.rules.level2.L2_01_numeric_purity import NumericPurityRule
rng=random.Random(201); cases=[]
values=['',' ', '100','3,500','100円','3kg','20%','５％','▲123','2(注)','-42','N/A','値','1x','0.5','1\u001c円','1\n円','１２','１２注','...']
for i in range(180):
    cols=rng.randint(1,4);rows=rng.randint(1,8);cells=[]
    schemas=[] if i%4==0 else [[c,rng.choice(['numeric','mixed','text','date','empty'])] for c in range(cols) if rng.random()<.8]
    for r in range(rows):
        for c in range(cols):
            if rng.random()<.8: cells.append([r,c,rng.choice(values)])
    mapping={(r+1,c+1):Cell(pos=CellPosition(row=r+1,col=c+1),value=v) for r,c,v in cells}
    sheet=Sheet(name='CSV',cells=mapping,max_row=rows,max_col=cols);workbook=WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
    columns=[ColumnSchema(col_index=c+1,inferred_type=kind) for c,kind in schemas]
    region=TableRegion(range=CellRange(start_row=1,start_col=1,end_row=rows,end_col=cols),layout=TableLayout(columns=columns,body_start_row=1,body_end_row=rows),confidence=1)
    result=NumericPurityRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region))
    cases.append(dict(rows=rows,cols=cols,schemas=schemas,cells=cells,passed=result.passed,confidence=result.confidence,findings=[[v.cell_range,v.severity] for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_') and k not in {'OPENAI_API_KEY','ANTHROPIC_API_KEY','GOOGLE_API_KEY','AZURE_OPENAI_API_KEY'}};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-numeric-purity-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
