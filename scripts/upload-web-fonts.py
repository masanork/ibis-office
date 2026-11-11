#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Upload verified sources to this project's R2 bucket."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--remote', action='store_true', help='use production R2 instead of local development storage')
args = parser.parse_args()
for font in json.loads((ROOT / 'fonts/sources.json').read_text())['fonts']:
    source = ROOT / 'data/fonts' / font['filename']
    if not source.is_file() or hashlib.sha256(source.read_bytes()).hexdigest() != font['sha256']:
        raise SystemExit('Run scripts/fetch-web-fonts.py first: ' + font['id'])
    subprocess.run([
        str(ROOT / 'node_modules/.bin/wrangler'), 'r2', 'object', 'put',
        'ibis-office-fonts/' + font['key'], '--file', str(source),
        '--content-type', 'font/otf' if source.suffix == '.otf' else 'font/ttf',
        '--remote' if args.remote else '--local',
    ], cwd=ROOT, check=True)
