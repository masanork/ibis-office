#!/usr/bin/env python3
"""Record single-table, unmerged L1-03 results from pinned Harunobu."""
import json, os, subprocess, sys
from pathlib import Path
commit = '88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo = Path(sys.argv[1]).resolve()
if subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip() != commit:
    raise SystemExit('Harunobu checkout is not at the pinned revision')
code = r'''
import json, random
from harunobu.core.models import Cell, CellPosition, Sheet, WorkBook, TableContext, TableRegion, CellRange
from harunobu.rules.level1.L1_03_data_fragmentation import DataFragmentationRule
rng = random.Random(3103)
cases = []
for i in range(160):
    rows = rng.randint(1, 8); cols = rng.randint(1, 7)
    r0 = rng.randint(0, 3); c0 = rng.randint(0, 3)
    values = ['',' ', '\u001c', '\u3000', '項目', '0', ' x ']
    cells = [[r0+r, c0+c, rng.choice(values)] for r in range(rows) for c in range(cols) if rng.random() < 0.5]
    mapping = {(r+1,c+1):Cell(pos=CellPosition(row=r+1,col=c+1),value=v) for r,c,v in cells}
    sheet = Sheet(name='CSV',cells=mapping,max_row=r0+rows,max_col=c0+cols)
    workbook = WorkBook(file_name='x.csv',file_format='csv',sheets=[sheet])
    region = TableRegion(range=CellRange(start_row=r0+1,start_col=c0+1,end_row=r0+rows,end_col=c0+cols),confidence=1.0)
    result = DataFragmentationRule().check(TableContext(workbook=workbook,sheet=sheet,table_region=region,all_tables=[region]))
    cases.append(dict(range=[r0,c0,r0+rows-1,c0+cols-1],cells=cells,passed=result.passed,confidence=result.confidence,locations=[v.cell_range for v in result.violations],severities=[v.severity for v in result.violations]))
print(json.dumps(cases,ensure_ascii=False))
'''
env = {k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')}; env['HARUNOBU_AI_DISABLED'] = '1'
run = subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code],text=True,capture_output=True,check=True,env=env)
out = Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability/harunobu-fragmentation-reference.json'
out.write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,separators=(',',':'))+'\n')
