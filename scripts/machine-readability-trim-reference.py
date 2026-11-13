#!/usr/bin/env python3
"""Record boundary trimming from a pinned local Harunobu checkout."""
import json
from pathlib import Path
import subprocess
import sys

commit = '88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo = Path(sys.argv[1]).resolve()
if subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip() != commit:
    raise SystemExit('Harunobu checkout is not at the pinned revision')
code = r'''
import json, random
from harunobu.core.layout.island import _trim_boundaries
from harunobu.core.models import Cell, CellPosition, CellFormat, Sheet
rng = random.Random(47199)
values = [None, None, '', 'data', '合計', '小計', '※注記', '注: 説明', '出典: 例', 'Source: x', ' 参考資料', '値', 123, True]
formulas = [None, None, None, '=SUM(A1:A2)', '=SUBTOTAL(9,A1:A2)', '=SUMMARY(A1)', '= SUM (A1)', '=1+1']
cases = []
for _ in range(300):
    height, width = rng.randrange(1, 9), rng.randrange(1, 13)
    raw = []
    for r in range(height):
        for c in range(width):
            if rng.random() >= 0.32: continue
            value = rng.choice(values)
            bold, merged = rng.random() < 0.08, rng.random() < 0.08
            formula = rng.choice(formulas) if value is not None else None
            raw.append(dict(row=r, column=c, value=value, bold=bold, merged=merged, formula=formula))
    cells = {(a['row']+1,a['column']+1): Cell(pos=CellPosition(row=a['row']+1,col=a['column']+1),value=a['value'],fmt=CellFormat(bold=a['bold']),is_merged=a['merged'],formula=a['formula']) for a in raw}
    sheet=Sheet(name='fixture',cells=cells,max_row=height,max_col=width)
    start,end,others = _trim_boundaries(sheet,1,height,1,width)
    cases.append(dict(height=height,width=width,cells=raw,trimmed=None if start>end else [start-1,end-1],others=[[o.row-1,o.col-1,o.region_type] for o in others]))
print(json.dumps(cases,ensure_ascii=False))
'''
run = subprocess.run([str(repo / '.venv/bin/python'), '-I', '-X', 'utf8', '-c', code], text=True, capture_output=True, check=True)
data = {'harunobu_commit': commit, 'cases': json.loads(run.stdout)}
out = Path(__file__).resolve().parent.parent / 'test/fixtures/machine-readability/harunobu-trim-reference.json'
out.write_text(json.dumps(data, ensure_ascii=False, separators=(',', ':')) + '\n')
