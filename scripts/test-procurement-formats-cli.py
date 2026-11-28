#!/usr/bin/env python3
"""End-to-end CLI regression tests for P0-P3 procurement formats using synthetic fixtures."""

from __future__ import annotations

import os
import struct
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path


def run_cli(binary: Path, input_path: Path, export_fmt: str, out_dir: Path, extra: list[str] | None = None) -> Path:
    cmd = [str(binary), str(input_path), "--export", export_fmt, "--out-dir", str(out_dir)]
    if extra:
        cmd.extend(extra)
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        raise AssertionError(
            f"CLI failed ({proc.returncode}): {' '.join(cmd)}\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
        )
    lines = [ln.strip() for ln in proc.stdout.splitlines() if ln.strip()]
    if not lines:
        raise AssertionError(f"No output path printed by CLI: {' '.join(cmd)}")
    return Path(lines[-1])


def build_minimal_cfb(streams: list[tuple[str, bytes]]) -> bytes:
    """Build a minimal valid OLE2 CFB v3 file with regular 512-byte sectors (pads streams >= 4096 bytes)."""
    sec = 512
    # Pad each stream to >= 4096 bytes so no mini-stream is needed.
    padded = [(name, data.ljust(max(4096, ((len(data) + 511) // 512) * 512), b"\x00"), len(data)) for name, data in streams]

    # Sector layout:
    # 0: FAT sector
    # 1: Directory sector (supports up to 4 entries: Root + 3 streams)
    # 2..: Stream sectors
    assert len(padded) <= 3, "This minimal helper supports up to 3 streams per CFB"

    fat = [0xFFFFFFFF] * 128
    fat[0] = 0xFFFFFFFD  # FATSECT
    fat[1] = 0xFFFFFFFE  # ENDOFCHAIN (Directory)

    dir_entries = bytearray(sec)
    stream_blobs = bytearray()
    cur_sec = 2

    def write_dir_entry(slot: int, name: str, etype: int, start_sec: int, size: int, left: int = -1, right: int = -1, child: int = -1) -> None:
        off = slot * 128
        raw_name = (name + "\x00").encode("utf-16le")
        dir_entries[off : off + len(raw_name)] = raw_name
        struct.pack_into("<H", dir_entries, off + 64, len(raw_name))
        dir_entries[off + 66] = etype
        dir_entries[off + 67] = 1  # black
        struct.pack_into("<iii", dir_entries, off + 68, left, right, child)
        struct.pack_into("<II", dir_entries, off + 116, start_sec, size)

    # Root entry points to child 1
    write_dir_entry(0, "Root Entry", 5, 0xFFFFFFFE, 0, child=1)

    for idx, (name, blob, orig_len) in enumerate(padded, start=1):
        n_secs = len(blob) // sec
        start_sec = cur_sec
        for s in range(n_secs):
            fat[cur_sec + s] = cur_sec + s + 1 if s + 1 < n_secs else 0xFFFFFFFE
        cur_sec += n_secs
        stream_blobs.extend(blob)
        left = -1
        right = idx + 1 if idx < len(padded) else -1
        write_dir_entry(idx, name, 2, start_sec, orig_len, left=left, right=right)

    header = bytearray(sec)
    header[0:8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"
    struct.pack_into("<HHHH", header, 24, 0x003E, 0x0003, 0xFFFE, 9)
    struct.pack_into("<H", header, 32, 6)
    struct.pack_into("<IIIIII", header, 44, 1, 1, 0, 0, 0xFFFFFFFE, 0)
    struct.pack_into("<II", header, 68, 0xFFFFFFFE, 0)
    difat = [0xFFFFFFFF] * 109
    difat[0] = 0
    struct.pack_into("<109I", header, 76, *difat)

    fat_bytes = struct.pack("<128I", *fat)
    return bytes(header) + fat_bytes + bytes(dir_entries) + bytes(stream_blobs)


def main() -> int:
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <path-to-ibisOffice>", file=sys.stderr)
        return 2
    binary = Path(sys.argv[1]).resolve()
    if not binary.exists():
        print(f"Binary not found: {binary}", file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory(prefix="ibis-reg-") as tmp:
        td = Path(tmp)
        out = td / "out"
        out.mkdir()

        # 1. P1: EML unpack + md
        eml = td / "sample.eml"
        eml.write_text(
            "From: sender@example.jp\r\nTo: recv@example.jp\r\nSubject: Test Notice\r\n"
            "Content-Type: text/plain; charset=utf-8\r\n\r\nHello EML\r\n",
            encoding="utf-8",
        )
        md_eml = run_cli(binary, eml, "md", out)
        assert "Test Notice" in md_eml.read_text(encoding="utf-8")

        # 2. P2: CALS/EC XML -> md
        cals = td / "INDEX_D.XML"
        cals.write_text(
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            "<report><基礎情報><工事名>テスト橋梁補修工事</工事名></基礎情報></report>\n",
            encoding="utf-8",
        )
        md_cals = run_cli(binary, cals, "md", out)
        assert "テスト橋梁補修工事" in md_cals.read_text(encoding="utf-8")

        # 3. P2: DXF -> svg + md
        dxf = td / "plan.dxf"
        dxf.write_text(
            "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0.0\n20\n0.0\n11\n100.0\n21\n50.0\n0\nENDSEC\n0\nEOF\n",
            encoding="utf-8",
        )
        svg_dxf = run_cli(binary, dxf, "svg", out)
        assert "<line" in svg_dxf.read_text(encoding="utf-8")
        md_dxf = run_cli(binary, dxf, "md", out)
        assert "DXF" in md_dxf.read_text(encoding="utf-8")

        # 4. P3: GeoJSON -> svg + md
        geojson = td / "map.geojson"
        geojson.write_text(
            '{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"庁舎"},"geometry":{"type":"Point","coordinates":[139.75,35.68]}}]}',
            encoding="utf-8",
        )
        svg_gj = run_cli(binary, geojson, "svg", out)
        assert "<circle" in svg_gj.read_text(encoding="utf-8")
        md_gj = run_cli(binary, geojson, "md", out)
        assert "庁舎" in md_gj.read_text(encoding="utf-8")

        # 5. P3: GTFS-JP ZIP -> md
        gtfs_zip = td / "gtfs_jp.zip"
        with zipfile.ZipFile(gtfs_zip, "w") as zf:
            zf.writestr("agency.txt", "agency_id,agency_name,agency_url,agency_timezone\nA1,市営バス,https://example.jp,Asia/Tokyo\n")
            zf.writestr("stops.txt", "stop_id,stop_name,stop_lat,stop_lon\nS1,市役所前,35.68,139.75\n")
            zf.writestr("routes.txt", "route_id,agency_id,route_short_name,route_long_name,route_type\nR1,A1,系統1,駅前〜市役所,3\n")
            zf.writestr("trips.txt", "route_id,service_id,trip_id\nR1,WD,T1\n")
            zf.writestr("stop_times.txt", "trip_id,arrival_time,departure_time,stop_id,stop_sequence\nT1,08:00:00,08:00:00,S1,1\n")
            zf.writestr("calendar.txt", "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\nWD,1,1,1,1,1,0,0,20260101,20261231\n")
            zf.writestr("routes_jp.txt", "route_id,route_update_date,origin_stop,via_stop,destination_stop\nR1,20260101,駅前,中央,市役所\n")
        md_gtfs = run_cli(binary, gtfs_zip, "md", out)
        gtfs_text = md_gtfs.read_text(encoding="utf-8")
        assert "GTFS-JP" in gtfs_text and "市営バス" in gtfs_text

        # 6. P3: Synthetic Hanako (.jhd) -> md + svg
        text_rec = bytearray(40)
        text_rec[0:4] = b"\x01\x00\x16\x60"
        struct.pack_into(">4i", text_rec, 8, 100, 200, 500, 400)
        text_rec.extend(b"\x00\x00\x00\x0d" + "回帰テスト花子".encode("utf-16be") + b"\x00\x0d\x00\x01")
        struct.pack_into(">I", text_rec, 4, len(text_rec))

        vec_rec = bytearray(32)
        vec_rec[0:4] = b"\x01\x00\x01\x60"
        struct.pack_into(">H", vec_rec, 4, 32)
        struct.pack_into(">4i", vec_rec, 16, 100, 200, 500, 400)

        jhd = td / "synthetic.jhd"
        jhd.write_bytes(build_minimal_cfb([("FDMText", bytes(text_rec)), ("FDMVector", bytes(vec_rec))]))
        md_jhd = run_cli(binary, jhd, "md", out)
        assert "回帰テスト花子" in md_jhd.read_text(encoding="utf-8")
        svg_jhd = run_cli(binary, jhd, "svg", out)
        svg_jhd_txt = svg_jhd.read_text(encoding="utf-8")
        assert "<line" in svg_jhd_txt and "回帰テスト花子" in svg_jhd_txt

        # 7. P3: Synthetic Sanshiro (.jsd) -> md
        sheet_rec = bytearray(10)
        struct.pack_into("<H", sheet_rec, 0, 0x000A)
        sheet_rec.extend("予算表".encode("utf-16le") + b"\x00\x00")
        struct.pack_into("<H", sheet_rec, 2, len(sheet_rec))
        book = b"\xff\xff" + bytes(sheet_rec)

        sub = bytearray(6)
        struct.pack_into("<H", sub, 0, 0x0001)
        struct.pack_into("<H", sub, 4, 0x0027)
        sub.extend("令和8年度予算".encode("utf-16le") + b"\x00\x00")
        struct.pack_into("<H", sub, 2, len(sub))

        ws = bytearray(12)
        struct.pack_into("<H", ws, 0, 0x1000)
        struct.pack_into("<IHH", ws, 4, 5, 0, 1)  # row 0 -> 1, col 1 -> B => B1
        ws.extend(sub)
        struct.pack_into("<H", ws, 2, len(ws))

        jsd = td / "synthetic.jsd"
        jsd.write_bytes(build_minimal_cfb([("Book", book), ("WorkSheet1", bytes(ws))]))
        md_jsd = run_cli(binary, jsd, "md", out)
        jsd_text = md_jsd.read_text(encoding="utf-8")
        assert "予算表" in jsd_text and "令和8年度予算" in jsd_text and "`B1`" in jsd_text

    print("All P0-P3 procurement format CLI regression checks passed (7/7).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
