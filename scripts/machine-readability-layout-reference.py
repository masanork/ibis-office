#!/usr/bin/env python3
"""Record island detector primitives from a pinned local Harunobu checkout."""
import json
from pathlib import Path
import random
import subprocess
import sys

commit = '88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
repo = Path(sys.argv[1]).resolve()
if subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip() != commit:
    raise SystemExit('Harunobu checkout is not at the pinned revision')
code = r'''
import json, random
from harunobu.core.layout.island import IslandDetector, _split_into_bands
from harunobu.core.models import Cell, CellPosition, Sheet
rng = random.Random(73899)
bands = []
for _ in range(200):
    positions = sorted(rng.sample(range(40), rng.randrange(20)))
    gap = rng.randrange(1, 5)
    bands.append(dict(indices=positions, gap=gap, expected=_split_into_bands(positions, gap)))
regions = []
for _ in range(150):
    height, width = rng.randrange(2, 10), rng.randrange(3, 12)
    occupied = {(r,c) for r in range(height) for c in range(width) if rng.random() < 0.55}
    # Prevent the reference's title and source trimming, which is a separate phase.
    occupied.update((r,c) for r in (0, height-1) for c in range(3))
    cells = {(r+1,c+1): Cell(pos=CellPosition(row=r+1,col=c+1), value='data') for r,c in occupied}
    sheet = Sheet(name='fixture', cells=cells, max_row=height, max_col=width)
    region, others = IslandDetector()._evaluate_region_with_trim(sheet, (1,height), (1,width))
    assert region and not others and region.range.start_row == 1 and region.range.end_row == height
    regions.append(dict(height=height, width=width, occupied=sorted(occupied), confidence=region.confidence))
print(json.dumps(dict(bands=bands, regions=regions), ensure_ascii=False))
'''
run = subprocess.run([str(repo / '.venv/bin/python'), '-I', '-X', 'utf8', '-c', code], text=True, capture_output=True, check=True)
data = {'harunobu_commit': commit, **json.loads(run.stdout)}
out = Path(__file__).resolve().parent.parent / 'test/fixtures/machine-readability/harunobu-layout-reference.json'
out.write_text(json.dumps(data, ensure_ascii=False, separators=(',', ':')) + '\n')
