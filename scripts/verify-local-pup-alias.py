#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (c) masanork
from fontTools.ttLib import TTFont
from fontTools.pens.recordingPen import DecomposingRecordingPen
import csv,sys
source=TTFont(sys.argv[1]);candidate=TTFont(sys.argv[2]);ivs=next(t for t in source['cmap'].tables if t.format==14).uvsDict;ivs={vs:dict(rows) for vs,rows in ivs.items()};a=source.getGlyphSet();b=candidate.getGlyphSet();src_base=source.getBestCmap();out=candidate.getBestCmap();seen=set();n=0
for row in csv.reader(open(sys.argv[3],encoding='utf-8-sig')):
 pup=ord(row[0]);cp,vs=map(ord,row[1][:2]);sg=ivs[vs][cp] or src_base[cp];tg=out[pup];key=(sg,tg)
 if key not in seen:
  left=DecomposingRecordingPen(a);right=DecomposingRecordingPen(b);a[sg].draw(left);b[tg].draw(right)
  if left.value!=right.value:raise ValueError(f'outline mismatch U+{pup:06X} {sg} {tg}')
  seen.add(key)
 n+=1
print('checked',n,'mappings;',len(seen),'unique decomposed glyph pairs match')
