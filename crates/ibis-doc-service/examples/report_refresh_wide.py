# SPDX-License-Identifier: AGPL-3.0-only
"""Check eight wide report aggregates against DuckDB and measure the macOS release probe.

Build report_refresh_probe with --release --features report-profiling, then run:
uv run --with duckdb --with pyarrow --with fastavro python report_refresh_wide.py /path/to/probe
"""

import argparse
import json
from pathlib import Path
import tempfile

import duckdb
import fastavro
import pyarrow as pa

from report_refresh_macos import measure

NUMBERS = ["amount", "units"] + [f"m{i}" for i in range(12)]
COLUMNS = ["id", "region", "code", "note", "tags", *NUMBERS]
QUERIES = [
    "SELECT count(*) AS n FROM facts",
    "SELECT sum(f.amount * d.factor) AS n FROM facts f JOIN dimension d ON f.region = d.region",
    """SELECT sum(subtotal * (region + 1)) AS n FROM (
        SELECT region, sum(units) AS subtotal FROM facts GROUP BY region
    ) AS grouped""",
    """SELECT sum(j.n * d.factor) AS n FROM (
        SELECT region, count(*) AS n FROM facts GROUP BY region
    ) j JOIN dimension d ON j.region = d.region""",
    """SELECT count(*) AS n FROM facts f JOIN json_each(f.tags) t ON true
       WHERE t.value = 'review'""",
    """SELECT sum(weighted) AS n FROM (
        SELECT t.value, sum(f.units * d.factor) AS weighted
        FROM facts f JOIN dimension d ON f.region = d.region
        JOIN json_each(f.tags) t ON true GROUP BY t.value
    ) AS groups""",
    "SELECT sum(coalesce(length(note), 0)) AS n FROM facts WHERE amount IS NULL",
    """SELECT sum(coalesce(m0, 0) + coalesce(m11, 0)) AS n FROM facts
       WHERE code LIKE '00%' AND length(note) > 500""",
]


def rows(start, stop):
    for i in range(start, stop):
        row = {
            "id": i, "region": i % 64, "code": f"{i % 101:04d}",
            "note": None if i % 19 == 0 else "注記" * (512 if i % 113 == 1 else 8),
            "tags": None if i % 23 == 0 else ["review" if i % 3 == 0 else "ready", "合成"],
        }
        row.update({
            name: None if (i + offset) % 41 == 0 else (i * (offset + 1) + offset) % 997
            for offset, name in enumerate(NUMBERS)
        })
        yield row


def write_sources(directory, count):
    directory.mkdir()
    fields = [(name, pa.int64()) for name in ["id", "region", *NUMBERS]]
    fields.extend([("code", pa.string()), ("note", pa.string()),
                   ("tags", pa.list_(pa.string()))])
    arrow_schema = pa.schema(fields)
    avro_schema = {
        "type": "record", "name": "WideFact", "fields": [
            {"name": name, "type": ["null", "string" if name in ("code", "note", "tags") else "long"]}
            for name in COLUMNS
        ],
    }
    with (directory / "facts.jsonl").open("w", encoding="utf-8") as text, \
         (directory / "facts.avro").open("wb") as avro, \
         pa.ipc.new_stream(directory / "facts.ipc", arrow_schema) as arrow:
        def avro_records():
            for start in range(0, count, 8192):
                batch = list(rows(start, min(start + 8192, count)))
                arrow.write_batch(pa.RecordBatch.from_pylist(batch, schema=arrow_schema))
                for row in batch:
                    print(json.dumps(row, ensure_ascii=False, separators=(",", ":")), file=text)
                    # The Avro SQL reader accepts text, while Arrow and JSON Lines accept lists.
                    if row["tags"] is not None:
                        row["tags"] = json.dumps(row["tags"], ensure_ascii=False, separators=(",", ":"))
                    yield row

        fastavro.writer(avro, avro_schema, avro_records())
    with (directory / "dimension.jsonl").open("w", encoding="utf-8") as file:
        for key in range(64):
            print(json.dumps({"region": key, "factor": 1 + key % 11}), file=file)


def reference(source):
    with duckdb.connect(config={"threads": 1}) as connection:
        connection.read_json(str(source / "facts.jsonl"), format="newline_delimited").create_view("facts")
        connection.read_json(str(source / "dimension.jsonl"), format="newline_delimited").create_view("dimension")
        return [
            connection.execute(sql.replace("json_each(f.tags)", "json_each(to_json(f.tags))")
                               .replace("t.value = 'review'", "t.value = '\"review\"'")).fetchone()[0]
            for sql in QUERIES
        ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=Path, help="Path to the release example executable")
    parser.add_argument("--rows", type=int, default=400_000)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="ibis-wide-report-") as directory:
        root = Path(directory)
        seed, source = root / "seed", root / "source"
        write_sources(seed, 128)
        write_sources(source, args.rows)
        workload = root / "workload.json"
        workload.write_text(json.dumps({"columns": COLUMNS, "queries": QUERIES,
                                        "dimension_file": "dimension.jsonl"}))
        expected = reference(source)
        print(json.dumps({"duckdb": duckdb.__version__, "rows": args.rows,
                          "columns": len(COLUMNS), "expected": expected}), flush=True)
        for filename in ["facts.ipc", "facts.jsonl", "facts.avro"]:
            result = measure(args.probe.resolve(strict=True), "shared", seed / filename,
                             source / filename, len(QUERIES), root, workload)
            if "sql_ms" not in result["timings"][0]:
                raise RuntimeError("Build report_refresh_probe with --features report-profiling")
            if args.rows * len(COLUMNS) > 100_000 and not result["sampled_peak_db_bytes"]:
                raise RuntimeError(f"{filename}: no temporary database was observed")
            if result["results"] != expected:
                raise RuntimeError(f"{filename}: {result['results']} != {expected}")
            print(json.dumps({"format": filename, "input_bytes": (source / filename).stat().st_size,
                              **result}), flush=True)


if __name__ == "__main__":
    main()
