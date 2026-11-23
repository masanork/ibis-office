# SPDX-License-Identifier: AGPL-3.0-only
"""Measure the release report_refresh_probe in independent macOS processes."""

import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import tempfile
import time


def temporary_database_bytes(root):
    total = 0
    for directory in root.glob("ibis-datasets-*"):
        try:
            for entry in directory.iterdir():
                try:
                    total += entry.stat().st_size
                except FileNotFoundError:
                    pass
        except FileNotFoundError:
            pass
    return total


def measure(probe, mode, seed, source, reports, scratch):
    with tempfile.TemporaryDirectory(dir=scratch) as directory:
        root = Path(directory)
        with subprocess.Popen(
            ["/usr/bin/time", "-l", probe, mode, seed, source, str(reports)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env={**os.environ, "TMPDIR": directory},
        ) as process:
            peak_disk = 0
            while process.poll() is None:
                peak_disk = max(peak_disk, temporary_database_bytes(root))
                time.sleep(0.005)
            stdout, stderr = process.communicate()
            if process.returncode:
                raise RuntimeError(stderr)
        if list(root.iterdir()):
            raise RuntimeError(f"Temporary files remain after {source.name} ({mode})")
        rss = int(re.search(r"(\d+)\s+maximum resident set size", stderr)[1])
        result = json.loads(stdout)
        return {
            **result,
            "peak_rss_bytes": rss,
            "sampled_peak_db_bytes": peak_disk,
        }


def summarize(runs):
    timing_fields = runs[0]["timings"][0].keys()
    summary = {
        field: [
            statistics.median(run["timings"][index][field] for run in runs)
            for index in range(2)
        ]
        for field in timing_fields
    }
    summary["peak_rss_bytes"] = statistics.median(run["peak_rss_bytes"] for run in runs)
    summary["sampled_peak_db_bytes"] = max(run["sampled_peak_db_bytes"] for run in runs)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=Path, help="Path to the release example executable")
    parser.add_argument("--rows", type=int, default=400_000)
    parser.add_argument("--reports", type=int, choices=range(2, 13), default=8)
    args = parser.parse_args()
    if args.rows <= 50_000:
        parser.error("--rows must exceed the in-memory cell limit for two columns")
    probe = args.probe.resolve(strict=True)
    groups_of_seven, remainder = divmod(args.rows, 7)
    expected = [args.rows] + [
        groups_of_seven * min(limit, 7) + min(remainder, limit)
        for limit in range(1, args.reports)
    ]
    with tempfile.TemporaryDirectory(prefix="ibis-report-measure-") as directory:
        root = Path(directory)
        seed, input_dir = root / "seed", root / "input"
        subprocess.run([probe, "generate", seed, "2"], check=True)
        subprocess.run([probe, "generate", input_dir, str(args.rows)], check=True)
        for filename, pairs in [("facts.ipc", 2), ("facts.jsonl", 2), ("facts.avro", 4)]:
            source = input_dir / filename
            runs = {"shared": [], "separate": []}
            for pair in range(pairs):
                order = list(runs)
                if pair % 2:
                    order.reverse()
                timings = {}
                for mode in order:
                    result = measure(probe, mode, seed / filename, source, args.reports, root)
                    if result["results"] != expected or not result["sampled_peak_db_bytes"]:
                        raise RuntimeError(f"Unexpected results for {filename} ({mode}): {result}")
                    runs[mode].append(result)
                    timings[mode] = result
                print(json.dumps({"file": filename, "pair": pair + 1, "order": order,
                                  "runs": timings}), flush=True)
            print(json.dumps({"file": filename, "rows": args.rows, "reports": args.reports,
                              "input_bytes": source.stat().st_size,
                              "summary": {mode: summarize(samples) for mode, samples in runs.items()}}),
                  flush=True)


if __name__ == "__main__":
    main()
