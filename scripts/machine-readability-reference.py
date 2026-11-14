#!/usr/bin/env python3
"""Record selected reference results from an explicitly pinned Harunobu checkout."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

COMMIT = "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f"
ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "test/fixtures/machine-readability"
repo = Path(sys.argv[1]).resolve()
head = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
if head != COMMIT:
    raise SystemExit(f"Expected Harunobu {COMMIT}; got {head}")
result = {"harunobu_commit": COMMIT, "harunobu_version": "0.2.2", "mode": "standard", "ai_disabled": True, "fixtures": {}}
with tempfile.TemporaryDirectory() as tmp:
    for key, file in [("structure", "structure.xlsx"), ("csv", "data.csv"), ("header", "header-merge.xlsx")]:
        out = Path(tmp) / "result.json"
        env = {**os.environ, "HARUNOBU_AI_DISABLED": "1"}
        subprocess.run([str(repo / ".venv/bin/harunobu"), "analyze", str(FIXTURES / file), "--mode", "standard", "--output", "json", "--out-path", str(out)], check=True, env=env)
        data = json.loads(out.read_text())
        rules = []
        for sheet in data["sheets"]:
            for table in sheet["評価対象エリア"]:
                for rule in table["評価内容"]:
                    if rule["ルールID"] in {"L1-12", "L1-13", "L2-06"}:
                        rules.append({"sheet": sheet["シート名"], "table": table["範囲"], "rule": rule["ルールID"], "passed": rule["合否"], "skipped": rule["判定対象外"], "severity": rule["重大度"], "locations": [v["セル範囲"] for v in rule["違反"]], "finding_severities": [v["違反重大度"] for v in rule["違反"]]})
        result["fixtures"][key] = {"format": data["フォーマット"], "rules": sorted(rules, key=lambda r: r["rule"])}
(FIXTURES / "harunobu-reference.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")

# Character detection is intentionally compared separately from table inference
# and file encoding, because the live editor examines the whole sheet.
character_cases = ["abc日本語123", "①", "⑳", "⓪", "㉑", "㊿", "Ⅰ-0", "ⅿ", "Ａ", "ｚ", "９", "㈱", "㋿", "〜", "～", "㍻", "︰", "㎡", "㎏㎥", "面積㎡／Ａ", "Ⓐ", "😀"]
character_script = """import json, sys
from harunobu.rules.level1.L1_08_platform_dependent_chars import PlatformDependentCharsRule
r = PlatformDependentCharsRule()
print(json.dumps([{'value': value, 'detected': bool(r._detect_in_cell(value))} for value in json.load(sys.stdin)], ensure_ascii=False))
"""
process = subprocess.run([str(repo / ".venv/bin/python"), "-I", "-X", "utf8", "-c", character_script],
    input=json.dumps(character_cases, ensure_ascii=False), text=True, capture_output=True, check=True)
characters = {"harunobu_commit": COMMIT, "cases": json.loads(process.stdout)}
(FIXTURES / "harunobu-character-reference.json").write_text(json.dumps(characters, ensure_ascii=False, indent=2) + "\n")

# Score aggregation is compared independently from detection and inference.
scoring_script = """import json
from harunobu.core.models import CheckResult
from harunobu.core.scorer import LevelScorer

def rule(rule_id, passed, severity=None, confidence=1.0):
    return dict(rule_id=rule_id, passed=passed, severity=severity, confidence=confidence)
patterns = [
    [], [rule('L1-01', True)], [rule('L1-01', False, 'fatal')],
    [rule('skip', False, 'fatal', 0.0)],
    [rule('a', False, 'critical'), rule('b', False, 'major'), rule('c', True, 'minor')],
    [rule('a', False, None), rule('b', False, 'info'), rule('skip', True, 'fatal', 0.0)],
    [rule('repeated', False, 'critical')] * 11,
    [rule('a', False, 'fatal'), rule('a', False, 'major'), rule('b', True, 'fatal')],
]
scorer = LevelScorer()
cases = []
for level in (1, 2, 3):
    for rules in patterns:
        result = scorer._score_level(level, [CheckResult(**r) for r in rules])
        cases.append(dict(level=level, rules=rules, expected=result.model_dump(mode='json')))
print(json.dumps(cases, ensure_ascii=False))
"""
process = subprocess.run([str(repo / ".venv/bin/python"), "-I", "-X", "utf8", "-c", scoring_script],
    text=True, capture_output=True, check=True)
scoring = {"harunobu_commit": COMMIT, "cases": json.loads(process.stdout)}
(FIXTURES / "harunobu-scoring-reference.json").write_text(json.dumps(scoring, ensure_ascii=False, indent=2) + "\n")

workbook_rules_script = """import json
from harunobu.core.models import Sheet, WorkBook, TableContext, TableRegion, CellRange
from harunobu.rules.level1.L1_01_file_format import FileFormatRule
from harunobu.rules.level1.L1_02_one_table_per_sheet import OneTablePerSheetRule
sheet = Sheet(name='Data')
formats = []
for fmt in ['xlsx', 'csv', 'tsv', 'xls']:
    workbook = WorkBook.model_construct(file_name='data.' + fmt, file_format=fmt, sheets=[sheet])
    current = TableRegion(range=CellRange(start_row=1, start_col=1, end_row=4, end_col=3), confidence=0.8)
    context = TableContext(workbook=workbook, sheet=sheet, table_region=current)
    result = FileFormatRule().check(context)
    formats.append(dict(format=fmt, passed=result.passed, confidence=result.confidence, locations=[v.cell_range for v in result.violations]))
patterns = [
    [(0, 0, 3, 2, 0.9)],
    [(0, 0, 3, 2, 0.9), (6, 0, 9, 2, 0.9)],
    [(0, 0, 3, 2, 0.2), (6, 0, 9, 2, 0.3)],
    [(0, 0, 3, 2, 0.2), (6, 0, 9, 2, 0.5)],
    [(0, 0, 3, 2, 0.5), (6, 0, 9, 2, 0.5)],
]
workbook = WorkBook(file_name='data.xlsx', file_format='xlsx', sheets=[sheet])
cases = []
for pattern in patterns:
    tables = [TableRegion(range=CellRange(start_row=a+1, start_col=b+1, end_row=c+1, end_col=d+1), confidence=q) for a,b,c,d,q in pattern]
    for index, current in enumerate(tables):
        context = TableContext(workbook=workbook, sheet=sheet, table_region=current, all_tables=tables)
        result = OneTablePerSheetRule().check(context)
        cases.append(dict(tables=pattern, current=index, passed=result.passed, confidence=result.confidence, locations=[v.cell_range for v in result.violations], severities=[v.severity for v in result.violations]))
print(json.dumps(dict(formats=formats, tables=cases), ensure_ascii=False))
"""
process = subprocess.run([str(repo / ".venv/bin/python"), "-I", "-X", "utf8", "-c", workbook_rules_script],
    text=True, capture_output=True, check=True)
workbook_rules = {"harunobu_commit": COMMIT, **json.loads(process.stdout)}
(FIXTURES / "harunobu-workbook-rules-reference.json").write_text(json.dumps(workbook_rules, ensure_ascii=False, indent=2) + "\n")

csv_diagnostics_script = r'''import json
from harunobu.core.reader import _build_csv_diagnostics
cases = [
    ("empty", "", ","), ("one", "a,b\n1,2\n", ","),
    ("bom", "\ufeffa,b\n1,2", ","), ("crlf", "a,b\r\n1,2\r\n", ","),
    ("quoted", "a,b\n1,\"a,b\"\n", ","), ("escaped", "a,b\n1,\"a\"\"\"b\"\n", ","),
    ("embedded", "a,b\n1,\"first\nsecond\"\n2,3\n", ","),
    ("embedded_crlf", "a,b\r\n1,\"first\r\nsecond\"\r\n2,3", ","),
    ("unclosed", "a,b\n1,\"first\nsecond\n", ","),
    ("quote_in_unquoted", "a,b\n1,a\"b\n", ","),
    ("trailing_junk", "a,b\n1,\"x\"q\n", ","),
    ("mismatch", "a,b\n1,2,3\n4\n", ","),
    ("blank", "a,b\n\n1,2\n", ","), ("single_newline", "\n", ","),
    ("tabs", "a\tb\r\n1\t\"x\ny\"\r\n", "\t"),
    ("trailing_delimiter", "a,b,\n1,2,", ","),
    ("many_bad_rows", "a,b\n" + "1,2,3\n" * 1005, ","),
    ("many_bad_fields", "a\n" + ",".join(['a\"b'] * 1050), ","),
]
for cp in range(0x1c, 0x20):
    cases.append((f"python_space_{cp:02x}", '"x"' + chr(cp) + '\n', ','))
import random
random_source = random.Random(20261113)
for index in range(100):
    alphabet = ['a', 'b', 'あ', ' ', '\u3000', '\t', ',', '\"', '\r', '\n', '\x1c']
    text = ''.join(random_source.choice(alphabet) for _ in range(random_source.randrange(0, 100)))
    cases.append((f"random_{index:03}", text, ',' if index % 2 == 0 else '\t'))
out=[]
for name,text,delimiter in cases:
    d = _build_csv_diagnostics(text, delimiter)
    samples=[dict(kind=x.issue_type, row=x.logical_row, column=x.col, start=x.physical_start_line,
        end=x.physical_end_line, fields=x.field_count, expected=x.expected_field_count) for x in d.issue_samples]
    out.append(dict(name=name, text=text, delimiter=delimiter, counts=d.issue_counts, samples=samples,
        expected_fields=d.expected_field_count, total_records=d.total_records, truncated=d.truncated, confidence=d.confidence))
print(json.dumps(out, ensure_ascii=False))
'''
process = subprocess.run([str(repo / ".venv/bin/python"), "-I", "-X", "utf8", "-c", csv_diagnostics_script],
    text=True, capture_output=True, check=True)
csv_reference = {"harunobu_commit": COMMIT, "cases": json.loads(process.stdout)}
(FIXTURES / "harunobu-csv-diagnostics-reference.json").write_text(json.dumps(csv_reference, ensure_ascii=False, separators=(',', ':')) + "\n")

csv_rules_script = """import json, sys
from harunobu.core.reader import _build_csv_diagnostics
from harunobu.core.models import Sheet, WorkBook, TableContext, TableRegion, CellRange
from harunobu.rules.level1.L1_14_csv_single_line import CsvSingleLineRule
from harunobu.rules.level1.L1_15_csv_quoting import CsvQuotingRule
cases = json.load(sys.stdin)
out = []
for case in cases:
    diagnostic = _build_csv_diagnostics(case['text'], case['delimiter'])
    sheet = Sheet(name='CSV', csv_diagnostics=diagnostic)
    workbook = WorkBook(file_name='data.csv', file_format='csv' if case['delimiter'] == ',' else 'tsv', sheets=[sheet])
    region = TableRegion(range=CellRange(start_row=1, start_col=1, end_row=2000, end_col=4), confidence=1.0)
    context = TableContext(workbook=workbook, sheet=sheet, table_region=region)
    results = []
    for rule in [CsvSingleLineRule(), CsvQuotingRule()]:
        r = rule.check(context)
        results.append(dict(rule=r.rule_id, passed=r.passed, confidence=r.confidence,
            locations=[v.cell_range for v in r.violations], severities=[v.severity for v in r.violations]))
    out.append(dict(name=case['name'], results=results))
print(json.dumps(out, ensure_ascii=False))
"""
process = subprocess.run([str(repo / ".venv/bin/python"), "-I", "-X", "utf8", "-c", csv_rules_script],
    input=json.dumps(csv_reference['cases'], ensure_ascii=False), text=True, capture_output=True, check=True)
csv_rules = {"harunobu_commit": COMMIT, "cases": json.loads(process.stdout)}
(FIXTURES / "harunobu-csv-rules-reference.json").write_text(json.dumps(csv_rules, ensure_ascii=False, separators=(',', ':')) + "\n")

character_rule_script = """import json, sys
from harunobu.core.models import Cell, CellPosition, Sheet, WorkBook, TableContext, TableRegion, CellRange
from harunobu.rules.level1.L1_08_platform_dependent_chars import PlatformDependentCharsRule
values = json.load(sys.stdin)
cells = {(i, 1): Cell(pos=CellPosition(row=i, col=1), value=v) for i,v in enumerate(values, start=1)}
sheet = Sheet(name='Cases', cells=cells, max_row=len(values), max_col=1)
region = TableRegion(range=CellRange(start_row=1, start_col=1, end_row=len(values), end_col=1), confidence=1.0)
cases = []
for label, fmt, encoding in [('xlsx','xlsx',None), ('utf8','csv','utf-8'), ('cp932','csv','cp932')]:
    workbook = WorkBook(file_name='data.'+fmt, file_format=fmt, encoding=encoding, sheets=[sheet])
    r = PlatformDependentCharsRule().check(TableContext(workbook=workbook, sheet=sheet, table_region=region))
    cases.append(dict(encoding=label, passed=r.passed, confidence=r.confidence,
        locations=[v.cell_range for v in r.violations], severities=[v.severity for v in r.violations]))
print(json.dumps(cases, ensure_ascii=False))
"""
character_values = character_cases + ['Ⅰ-0', '  Ⅱ－１  ', 'Ⅰ-0x', '①Ａ', '㎡', '①㎡', 'ＡＢ９', '㋿']
process = subprocess.run([str(repo / ".venv/bin/python"), "-I", "-X", "utf8", "-c", character_rule_script],
    input=json.dumps(character_values, ensure_ascii=False), text=True, capture_output=True, check=True)
character_rule = {"harunobu_commit": COMMIT, "values": character_values, "cases": json.loads(process.stdout)}
(FIXTURES / "harunobu-character-rule-reference.json").write_text(json.dumps(character_rule, ensure_ascii=False, indent=2) + "\n")

# L1-07 is compared independently of layout inference. Column type is supplied
# explicitly so the numeric-column exception can be checked in isolation.
one_data_script = r'''import json
from harunobu.rules.level1.L1_07_one_data_per_cell import OneDataPerCellRule
values = ["", " ", "普通の文字", "a\nb", "a\n\n", "1（2）", " １（２） ", "A、B、C", "A、B", "A・B・C", "A・B", "a,b", "1,2", "1,234", "12,345,678.90", "1234,567", "1,23", "1,2,3", "1,２", "１,２", "a,1", "1,a", ",", "1,", ",2", "a （x,y）", "1（2,3）", "1, 2", "1,\t2", "a\n b, c", "(1,2)", "1.0,2.0", "001,002", "١,٢", "😀,😀", "\u001c1（2）\u001c", "1,\u001c2", "\u001c\na"]
r = OneDataPerCellRule()
print(json.dumps([dict(value=v, column=c, reason=r._has_multiple_data(v, col_type=c)) for v in values for c in [None, 'text', 'numeric']], ensure_ascii=False))
'''
process = subprocess.run([str(repo / '.venv/bin/python'), '-I', '-X', 'utf8', '-c', one_data_script], text=True, capture_output=True, check=True)
(FIXTURES / 'harunobu-one-data-reference.json').write_text(json.dumps({'harunobu_commit': COMMIT, 'cases': json.loads(process.stdout)}, ensure_ascii=False, indent=2) + '\n')
