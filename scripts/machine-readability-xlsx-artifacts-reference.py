#!/usr/bin/env python3
"""Capture formula artifact inspection from pinned Harunobu."""
import json,os,re,subprocess,sys,zipfile
from pathlib import Path
commit='88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f';repo=Path(sys.argv[1]).resolve()
if subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()!=commit: raise SystemExit('Harunobu checkout is not at the pinned revision')
root=Path(__file__).resolve().parent.parent/'test/fixtures/machine-readability'
with zipfile.ZipFile(root/'structure.xlsx') as src, zipfile.ZipFile(root/'xlsx-artifact-errors.xlsx','w') as dst:
    for info in src.infolist():
        data=src.read(info.filename)
        if info.filename=='xl/worksheets/sheet1.xml':
            text=data.decode('utf8')
            for ref,replacement in [('A2','<c r="A2" t="e"><v>#REF!</v></c>'),('C5','<c r="C5" t="e"><f>SUM(C2:C4)</f><v>#REF!</v></c>')]:
                text,count=re.subn(r'<c\b[^>]*\br="'+ref+r'"[^>]*>.*?</c>',replacement,text)
                assert count==1
            data=text.encode('utf8')
        dst.writestr(info,data)
code=r'''
import io,json,sys
from pathlib import Path
from harunobu.core.ooxml_inspector import inspect_formula_artifacts
from harunobu.core.reader import read_workbook
from harunobu.rules.level2.L2_06_formula_to_value import FormulaToValueRule
from harunobu.core.models import CellRange
from types import SimpleNamespace as S
from openpyxl import Workbook
workbook=Workbook();first=workbook.active;first.title='First';first['B2']='=1+1';first['A3']='#REF!'
second=workbook.create_sheet('Second');second['C3']='=2+2';second['D4']='#REF!'
workbook.save(Path(sys.argv[1])/'xlsx-artifact-multisheet.xlsx')
out=[]
for name in ['structure.xlsx','xlsx-cache-missing.xlsx','xlsx-cache-numeric.xlsx','xlsx-cache-boolean.xlsx','xlsx-cache-text.xlsx','xlsx-cache-formula_text.xlsx','xlsx-cache-error.xlsx','xlsx-artifact-errors.xlsx','xlsx-artifact-multisheet.xlsx']:
    result=inspect_formula_artifacts(Path(sys.argv[1])/name)
    assert result.error is None
    def conv(ref): return [ref.sheet_name,ref.row-1,ref.col-1,ref.cell_ref,getattr(ref,"formula_text",None),getattr(ref,"formula_type",None)]
    sheets=[s.name for s in read_workbook(name,io.BytesIO((Path(sys.argv[1])/name).read_bytes())).sheets]
    reports=[]
    for name_ in sheets:
        sheet=S(name=name_,formula_refs_loaded=True,formula_refs=[v for v in result.formula_refs if v.sheet_name==name_],formula_error_refs=[v for v in result.formula_error_refs if v.sheet_name==name_])
        report=FormulaToValueRule().check(S(sheet=sheet,table_region=S(range=CellRange(start_row=1,start_col=1,end_row=11,end_col=11))))
        reports.append(dict(message=report.message,descriptions=[v.description for v in report.violations]))
    out.append(dict(file=name,sheets=sheets,formulas=[conv(v) for v in result.formula_refs],errors=[conv(v) for v in result.formula_error_refs],reports=reports))
print(json.dumps(out,ensure_ascii=False))
'''
env={k:v for k,v in os.environ.items() if not k.startswith('HARUNOBU_')};env['HARUNOBU_AI_DISABLED']='1'
run=subprocess.run([str(repo/'.venv/bin/python'),'-I','-X','utf8','-c',code,str(root)],text=True,capture_output=True,check=True,env=env)
(root/'harunobu-xlsx-artifacts-reference.json').write_text(json.dumps({'harunobu_commit':commit,'cases':json.loads(run.stdout)},ensure_ascii=False,indent=2)+'\n')
