#!/usr/bin/env python3
"""Extract runtime region-name sets from the pinned Harunobu resource."""
import json, subprocess, sys
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'; repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
data=json.loads((repo/'harunobu/resources/region_dict.json').read_text())
groups={'P':{x['name'] for x in data['prefectures']},'S':{x['short'] for x in data['prefectures']},'M':{x['name'] for x in data['municipalities']},'A':{name for x in data['municipalities'] for name in x['aliases']}}
if any('\n' in name or '\t' in name for values in groups.values() for name in values): raise SystemExit('Unexpected delimiter in source data')
out=Path(__file__).resolve().parent.parent/'crates/ibis-doc-model/resources/harunobu-region-names.tsv'
out.write_text(''.join(f'{tag}\t{name}\n' for tag,values in groups.items() for name in sorted(values)),encoding='utf8')
