#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Read-only JTD audit, independently derived from published format notes.

Requires olefile; --pdf additionally requires PyMuPDF. Prints counts, offsets,
and hashes, never document text. This is a diagnostic, not an importer or a
claim that the reverse-engineered specification is complete. See
docs/jtd-decoding-audit.md for provenance and interpretation.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import struct
import unicodedata
import xml.etree.ElementTree as ET
import zipfile


def be32(data, offset):
    return struct.unpack_from(">I", data, offset)[0]


def substreams(data):
    if data[:8] != b"SsmgV.01" or len(data) < 20:
        raise ValueError("not an SsmgV.01 block store")
    count, size, blocks = struct.unpack_from(">III", data, 8)
    if not 0 < size <= 65536 or not 0 < count <= 65536:
        raise ValueError("unsupported block-store dimensions")
    offset = 20 + size * blocks
    if offset > len(data):
        raise ValueError("block area exceeds stream")
    result, directory = {}, []
    for _ in range(count):
        index, reserved, length, n, repeated = struct.unpack_from(">IIIII", data, offset)
        offset += 20
        if reserved or n != repeated or length > n * size or n > blocks:
            raise ValueError("invalid sub-stream directory")
        ids = struct.unpack_from(f">{n}I", data, offset)
        offset += 4 * n
        if index in result or any(i >= blocks for i in ids) or len(set(ids)) != n:
            raise ValueError("duplicate index/block or block out of range")
        result[index] = b"".join(data[20 + i * size:20 + (i + 1) * size] for i in ids)[:length]
        directory.append({"index": index, "bytes": length, "blocks": n,
                          "physical_order_contiguous": all(b == a + 1 for a, b in zip(ids, ids[1:]))})
    if data[offset:] != b"\0\0\0\0":
        raise ValueError("directory does not end exactly at stream terminator")
    return result, {"block_size": size, "block_count": blocks, "directory": directory}


def style_events(data, expected):
    i, covered, events = 0, 0, 0
    properties = Counter()
    while i < len(data):
        tag = data[i]
        i += 1
        if tag == 0:
            covered += be32(data, i)
            i += 4
        elif tag == 0xfe:
            while data[i] != 0xff:
                prop, size = data[i:i + 2]
                i += 2
                if i + size > len(data):
                    raise ValueError("truncated style property")
                properties[prop] += 1
                i += size
            if data[i:i + 2] != b"\xff\0":
                raise ValueError("invalid style event footer")
            i += 2
            covered += 1
            events += 1
        elif tag == 0xff:
            if covered != expected or i != len(data):
                raise ValueError("style coverage or end boundary mismatch")
            return {"covered_units": covered, "change_events": events,
                    "properties": dict(sorted(properties.items()))}
        else:
            raise ValueError("unknown style event")
        if covered > expected:
            raise ValueError("style coverage exceeds text")
    if covered == expected:
        return {"covered_units": covered, "change_events": events,
                "properties": dict(sorted(properties.items())), "end": "substream_boundary"}
    raise ValueError("style coverage does not reach text length")


def logical_text(data):
    streams, store = substreams(data)
    root = streams[0]
    if root[:8] == b"TextV.01":
        count = be32(root, 8)
        pieces = [(count, root[12:])]
        kind = "TextV.01"
    elif root[:8] == b"QLSTV.01":
        n = be32(root, 8)
        if len(root) not in (12 + 8 * n, 16 + 8 * n):
            raise ValueError("invalid piece list size")
        pairs = [struct.unpack_from(">II", root, 12 + 8 * i) for i in range(n)]
        pieces = [(count, streams[index]) for count, index in pairs]
        kind = "QLSTV.01"
    else:
        raise ValueError("unknown logical stream type")
    units, styles = [], []
    for count, piece in pieces:
        if count * 2 > len(piece):
            raise ValueError("piece text exceeds sub-stream")
        units.extend(struct.unpack_from(f">{count}H", piece))
        try:
            styles.append(style_events(piece[count * 2:], count))
        except (ValueError, IndexError, struct.error) as exc:
            # Unknown style variants must remain visible as audit failures;
            # they do not change the independently bounded text region.
            styles.append({"error": str(exc), "style_bytes": len(piece) - count * 2})
    return units, {**store, "kind": kind, "pieces": len(pieces),
                   "piece_list_trailing_word": be32(root, 12 + 8 * n) if kind == "QLSTV.01" and len(root) == 16 + 8 * n else None,
                   "text_units": len(units), "styles": styles}


def text_value(units):
    return b"".join(struct.pack(">H", u) for u in units).decode("utf-16-be")


def ruled_points(values):
    if len(values) < 3 or values[1] != 0:
        raise ValueError("invalid rule prefix")
    width, _, origin = values[:3]
    cursor, i, count, attached = origin + 1, 3, 0, 0
    while i < len(values):
        remaining = len(values) - i
        if remaining == 2:
            cursor += values[i + 1] + 1
            i += 2
        elif remaining >= 4:
            _, a, b, distance = values[i:i + 4]
            attached += bool(b & 0x30)
            cursor += a + distance + 2
            i += 4
        else:
            raise ValueError("invalid rule item length")
        count += 1
    if cursor != width:
        raise ValueError("rule width mismatch")
    return count, attached


def tokenize(units):
    """Strict record boundaries; plain text resumes after an inline footer."""
    i, text, ruby = 0, [], []
    records, controls, tags = Counter(), Counter(), Counter()
    rules, rejected_rules, legacy_rules, points, attached = 0, 0, 0, 0, 0
    inline_tails, tail_units, inline_controls = 0, [], Counter()
    while i < len(units):
        u = units[i]
        if u == 0x1c:
            start = i
            cls, length = units[i + 1:i + 3]
            records[cls] += 1
            if cls == 1:
                if length != 7 or units[i + 3] != 0 or units[i + 6] != 0x1d:
                    raise ValueError(f"invalid inline header at unit {i}")
                end = units.index(0x1e, i + 7)
                if units[end:end + 5] != [0x1e, 5, 0, 1, 0x1f]:
                    raise ValueError(f"invalid inline footer at unit {end}")
                value = text_value(units[i + 7:end])
                inline_controls.update(ord(c) for c in value if ord(c) < 0x20 and c not in '\t\n\r')
                if any(ord(c) < 0x20 and c not in '\t\n\r' for c in value):
                    value = ''  # Preserve/count field controls; do not invent visible text.
                selector = units[i + 4:i + 6]
                if selector in ([0, 1], [0, 3]):
                    text.append(value)
                elif selector == [1, 0x82]:
                    ruby.append(value)
                i = end + 5
                if i < len(units) and units[i] >= 0x20:
                    inline_tails += 1
                    tail_end = i
                    while tail_end < len(units) and (units[tail_end] >= 0xa0 or
                          0x20 <= units[tail_end] < 0x7f or units[tail_end] in (9, 10, 13)):
                        tail_end += 1
                    tail_units.extend(units[i:tail_end])
                continue
            end = i + length
            if length < 7 or end > len(units) or units[end - 4:end] != [length, 0, cls, 0x1f]:
                raise ValueError(f"invalid ordinary record at unit {i}")
            if cls == 0x10:
                if units[i + 3] != 0:
                    raise ValueError("unknown paragraph header prefix")
                p = i + 4
                while units[p:p + 2] != [0xffff, 0]:
                    tag, count = units[p:p + 2]
                    p += 2
                    if p + count > end - 6:
                        raise ValueError("TLV exceeds paragraph header")
                    tags[tag] += 1
                    if tag == 0x8f:
                        rules += 1
                        legacy_rules += p == start + 6 and count + 12 == length
                        try:
                            n, a = ruled_points(units[p:p + count])
                            points += n
                            attached += a
                        except ValueError:
                            rejected_rules += 1
                    p += count
                if p + 2 != end - 4:
                    raise ValueError("paragraph TLV terminator mismatch")
            i = end
        elif u < 0x20:
            controls[u] += 1
            if u in (9, 10, 13):
                text.append(chr(u))
            i += 1
        else:
            end = i + 1
            while end < len(units) and units[end] >= 0x20:
                end += 1
            text.append(text_value(units[i:end]))
            i = end
    return "".join(text), "".join(ruby), {
        "records": {f"{k:04x}": v for k, v in sorted(records.items())},
        "controls": {f"{k:04x}": v for k, v in sorted(controls.items())},
        "paragraph_tags": {f"{k:04x}": v for k, v in sorted(tags.items())},
        "ruled_headers": rules, "ruled_headers_matching_legacy_fixed_offsets": legacy_rules,
        "invalid_rule_payloads": rejected_rules, "rule_items": points,
        "attached_vertical_rules": attached, "inline_footers_followed_by_plain_text": inline_tails,
        "plain_text_after_inline_chars": len(normalized(text_value(tail_units))),
        "inline_control_units": {f"{k:04x}": v for k, v in sorted(inline_controls.items())},
    }


def normalized(text):
    return "".join(c for c in unicodedata.normalize("NFKC", text) if not c.isspace())


def compare(a, b):
    a, b = normalized(a), normalized(b)
    missing, extra = Counter(a) - Counter(b), Counter(b) - Counter(a)
    return {"source_chars": len(a), "candidate_chars": len(b), "same_sequence": a == b,
            "missing_char_occurrences": sum(missing.values()),
            "extra_char_occurrences": sum(extra.values())}


def docx_text(path):
    with zipfile.ZipFile(path) as z:
        root = ET.fromstring(z.read("word/document.xml"))
    ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
    # Ruby annotations are excluded from the base-text comparison.
    for annotation in root.findall(".//w:rt", ns):
        annotation.clear()
    rows = root.findall(".//w:tr", ns)
    return "".join(t.text or "" for t in root.findall(".//w:t", ns)), {
        "tables": len(root.findall(".//w:tbl", ns)), "rows": len(rows),
        "cells_per_row_distribution": dict(sorted(Counter(len(row.findall("w:tc", ns)) for row in rows).items())),
    }


def audit(path, pdf=None, docx=None, external_json=None):
    import olefile
    with olefile.OleFileIO(path) as container:
        stream = container.openstream("DocumentText").read()
    units, store = logical_text(stream)
    text, ruby, grammar = tokenize(units)
    result = {"sha256": hashlib.sha256(Path(path).read_bytes()).hexdigest(),
              "stream_bytes": len(stream), "store": store, "grammar": grammar,
              "base_chars": len(normalized(text)), "ruby_chars": len(normalized(ruby))}
    if docx:
        converted, model = docx_text(docx)
        with zipfile.ZipFile(docx) as archive:
            document = ET.fromstring(archive.read("word/document.xml"))
        ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
        readings = ''.join(n.text or '' for n in document.findall('.//w:rt//w:t', ns))
        result["docx"] = {**model, "base_text_comparison": compare(text, converted),
                          "ruby_text_comparison": compare(ruby, readings)}
    if external_json:
        external = json.loads(Path(external_json).read_text())
        def texts(value):
            if isinstance(value, dict):
                for key, item in value.items():
                    if key == "text" and isinstance(item, str):
                        yield item
                    elif key in ("runs", "rows", "cells", "blocks", "paragraphs"):
                        yield from texts(item)
            elif isinstance(value, list):
                for item in value:
                    yield from texts(item)
        # EZPZ JSON is only a second implementation's result, not an oracle.
        candidate = "".join(t for sheet in external["sheets"] for t in texts(sheet["blocks"]))
        controls = Counter(ord(c) for c in candidate if ord(c) < 0x20 and not c.isspace())
        result["external_json"] = {"raw": compare(text, candidate),
            "non_whitespace_c0": {f"{k:04x}": v for k, v in sorted(controls.items())},
            "without_c0": compare(text, "".join(c for c in candidate if ord(c) >= 0x20 or c.isspace()))}
    if pdf:
        import fitz
        with fitz.open(pdf) as document:
            extracted = "".join(page.get_text() for page in document)
            sizes = Counter()
            for page in document:
                for block in page.get_text("dict")["blocks"]:
                    for line in block.get("lines", []):
                        for span in line["spans"]:
                            sizes[round(span["size"], 2)] += len(span["text"])
            result["pdf"] = {"pages": len(document),
                "page_sizes_points": sorted(set((p.rect.width, p.rect.height) for p in document)),
                "font_size_char_counts": dict(sorted(sizes.items())),
                "comparison_including_ruby": compare(text + ruby, extracted)}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("jtd", type=Path)
    parser.add_argument("--pdf", type=Path)
    parser.add_argument("--docx", type=Path)
    parser.add_argument("--external-json", type=Path)
    args = parser.parse_args()
    print(json.dumps(audit(args.jtd, args.pdf, args.docx, args.external_json), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
