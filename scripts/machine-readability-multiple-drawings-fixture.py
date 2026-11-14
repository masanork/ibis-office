#!/usr/bin/env python3
"""Build a synthetic multi-drawing workbook for reference inspector comparison."""
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

root = Path(__file__).resolve().parent.parent / 'test/fixtures/machine-readability'
source = root / 'xlsx-chart-anchors.xlsx'
target = root / 'xlsx-multiple-drawings.xlsx'
with ZipFile(source) as src, ZipFile(target, 'w', ZIP_DEFLATED) as out:
    for item in src.infolist():
        data = src.read(item.filename)
        if item.filename == 'xl/worksheets/_rels/sheet1.xml.rels':
            data = data.replace(b'</Relationships>', b'<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="/xl/drawings/drawing2.xml" Id="rId2" /></Relationships>')
        out.writestr(item, data)
    drawing = src.read('xl/drawings/drawing1.xml')
    out.writestr('xl/drawings/drawing2.xml', drawing.replace(b'<row>1</row>', b'<row>4</row>').replace(b'<col>1</col>', b'<col>3</col>'))
