#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (c) masanork
"""Local IVS-to-PUP alias proof. Inputs and output remain local."""
import csv, sys, hashlib
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools import subset
from pup_combined import shaped_aliases
if len(sys.argv) not in (4,5):
 raise SystemExit('usage: build-local-pup-alias.py IPAMJ_TTF PUP_CSV OUTPUT_TTF [PUP_COMBINED_CSV]')
font_path,csv_path,out=map(Path,sys.argv[1:4]);font=TTFont(font_path);cmap14=next((t for t in font['cmap'].tables if t.format==14),None)
if cmap14 is None:raise SystemExit('font has no cmap14; refusing to substitute base glyph')
ivs={vs:dict(records) for vs,records in cmap14.uvsDict.items()};base=font.getBestCmap();aliases={}
for rowno,row in enumerate(csv.reader(csv_path.open(encoding='utf-8-sig')),1):
 if len(row)!=2 or len(row[0])!=1 or len(row[1])<2:raise ValueError(f'bad CSV row {rowno}')
 pup=ord(row[0]);chars=list(map(ord,row[1]));cp,vs=chars[:2]
 if not (0x101000<=pup<=0x104295 and 0xe0100<=vs<=0xe01ef):raise ValueError(f'invalid mapping row {rowno}')
 if pup in aliases:raise ValueError(f'duplicate PUP {pup:x}')
 if cp not in ivs.get(vs,{}):raise ValueError(f'IVS not in font at row {rowno}: {cp:x} {vs:x}')
 glyph=ivs[vs][cp] or base.get(cp)
 if not glyph or glyph=='.notdef':raise ValueError(f'no glyph at row {rowno}')
 if pup in base:raise ValueError(f'PUP collides with source cmap at row {rowno}')
 aliases[pup]=glyph
if len(sys.argv)==5:
 for pup,glyph in shaped_aliases(font_path,sys.argv[4],font):
  if pup in aliases or pup in base:raise ValueError(f'PUP collision U+{pup:X}')
  aliases[pup]=glyph
if not aliases:raise ValueError('no mapping rows')
if len(set(aliases.values())) != len(aliases):
 raise ValueError('multiple PUPs share a glyph; PDF text could not preserve their distinct codes')
for table in font['cmap'].tables:
 if table.format==12 and table.isUnicode():table.cmap.update(aliases)
if not all(font.getBestCmap().get(cp)==name for cp,name in aliases.items()):raise ValueError('alias cmap did not take effect')
options=subset.Options();options.layout_features=['*'];options.name_IDs=['*'];options.name_legacy=True;options.name_languages=['*'];sub=subset.Subsetter(options=options);sub.populate(unicodes=aliases);sub.subset(font)
# Local derivative must not present itself as the original font.
for record in font['name'].names:
 label={1:'Ibis PUP Alias Test',2:'Regular',3:'Ibis PUP Alias Test 1',4:'Ibis PUP Alias Test',6:'IbisPUPAliasTest',16:'Ibis PUP Alias Test',17:'Regular'}.get(record.nameID)
 if label is not None:
  try:record.string=label.encode(record.getEncoding())
  except (LookupError,UnicodeEncodeError):continue
font.save(out);result=TTFont(out);outmap=result.getBestCmap()
if set(aliases)!=set(outmap):raise ValueError('subset cmap mismatch')
if len(set(outmap.values())) != len(outmap):raise ValueError('subset merged PUP glyphs')
print('source_rows',len(aliases),'output_mappings',len(outmap),'bytes',out.stat().st_size)
print('input sha256',hashlib.sha256(font_path.read_bytes()).hexdigest(),'csv sha256',hashlib.sha256(csv_path.read_bytes()).hexdigest(),'output sha256',hashlib.sha256(out.read_bytes()).hexdigest())
