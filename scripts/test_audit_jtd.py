# SPDX-License-Identifier: AGPL-3.0-only
"""Synthetic grammar checks for the independent audit (no private fixtures)."""
import importlib.util
from pathlib import Path
import struct
import sys
import unittest

spec = importlib.util.spec_from_file_location("audit_jtd", Path(__file__).with_name("audit-jtd.py"))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def words(values):
    return b"".join(struct.pack(">H", x) for x in values)


def style_run(count):
    return b"\0" + struct.pack(">I", count)


def block_store(streams, size=256, reverse=False):
    blocks, directory = [], []
    for index, stream in streams.items():
        ids = []
        for offset in range(0, len(stream), size):
            ids.append(len(blocks))
            blocks.append(stream[offset:offset + size].ljust(size, b"\0"))
        directory.append((index, len(stream), ids))
    if reverse:
        blocks.reverse()
        directory = [(i, n, [len(blocks) - 1 - b for b in ids]) for i, n, ids in directory]
    result = b"SsmgV.01" + struct.pack(">III", len(streams), size, len(blocks)) + b"".join(blocks)
    for index, length, ids in directory:
        result += struct.pack(">IIIII", index, 0, length, len(ids), len(ids))
        result += struct.pack(f">{len(ids)}I", *ids)
    return result + b"\0\0\0\0"


def single_piece(units):
    return b"TextV.01" + struct.pack(">I", len(units)) + words(units) + style_run(len(units))


def probe_cfb(stream):
    """Minimal synthetic CFB v3, using a regular stream (no sample metadata)."""
    free, end, fat_sector = 0xffffffff, 0xfffffffe, 0xfffffffd
    if len(stream) < 4096:
        raise ValueError("probe must use regular-sector storage")
    sectors = (len(stream) + 511) // 512
    if sectors > 125:
        raise ValueError("probe exceeds one FAT sector")
    header = bytearray(512)
    header[:8] = bytes.fromhex("d0cf11e0a1b11ae1")
    struct.pack_into("<HHHHH", header, 24, 0x3e, 3, 0xfffe, 9, 6)
    struct.pack_into("<IIIIIIIII", header, 40, 0, 1, sectors, 0, 4096, end, 0, end, 0)
    struct.pack_into("<109I", header, 76, sectors + 1, *([free] * 108))
    def entry(name, kind, child, start, length):
        result = bytearray(128)
        encoded = (name + "\0").encode("utf-16-le")
        result[:len(encoded)] = encoded
        struct.pack_into("<HBBIII", result, 64, len(encoded), kind, 1, free, free, child)
        struct.pack_into("<IQ", result, 116, start, length)
        return result
    directory = entry("Root Entry", 5, 1, end, 0)
    directory += entry("DocumentText", 2, free, 0, len(stream))
    directory += bytes(256)
    fat = list(range(1, sectors)) + [end, end, fat_sector]
    fat += [free] * (128 - len(fat))
    return bytes(header) + stream.ljust(sectors * 512, b"\0") + directory + struct.pack("<128I", *fat)


def write_probes(destination):
    """Generate only synthetic files for testing the real Rust importer."""
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    inline = [0x1c, 1, 7, 0, 0, 3, 0x1d, ord("甲"), 0x1e, 5, 0, 1, 0x1f, ord("乙"), ord("丙")]
    text = [0x1c, 0x10, 10, 0, 0xffff, 0, 10, 0, 0x10, 0x1f]
    text += [ord("甲")] * 130 + [ord("乙")] * 10
    for name, units, reverse in [("inline", inline, False), ("ordered", text, False), ("reordered", text, True)]:
        # Unused 256-byte blocks keep the CFB stream outside the mini-stream.
        compact = block_store({0: single_piece(units)}, reverse=reverse)
        n = struct.unpack_from(">I", compact, 16)[0]
        body, directory = compact[20:20 + n * 256], compact[20 + n * 256:]
        stream = compact[:16] + struct.pack(">I", n + 15) + body + bytes(15 * 256) + directory
        (destination / f"{name}.jtd").write_bytes(probe_cfb(stream))


class AuditTests(unittest.TestCase):
    def test_physical_order_and_padding_cannot_change_text(self):
        units = list("甲乙丙".encode("utf-16-be"))
        units = list(struct.unpack(">3H", bytes(units)))
        # More than one block, with a text-like decoy in an unused sub-stream.
        source = single_piece(units * 100)
        normal = block_store({0: source, 1: single_piece([ord("偽")])})
        shuffled = block_store({0: source, 1: single_piece([ord("偽")])}, reverse=True)
        self.assertEqual(audit.logical_text(normal)[0], units * 100)
        self.assertEqual(audit.logical_text(normal)[0], audit.logical_text(shuffled)[0])

    def test_piece_list_can_split_inline_record(self):
        units = [0x1c, 1, 7, 0, 0, 3, 0x1d, ord("甲"), 0x1e, 5, 0, 1, 0x1f, ord("乙")]
        pieces = {1: units[:9], 2: units[9:]}
        root = b"QLSTV.01" + struct.pack(">I", 2)
        for index, piece in pieces.items():
            root += struct.pack(">II", len(piece), index)
        root += struct.pack(">I", 1)  # Observed extra word; semantic meaning unknown.
        streams = {0: root, **{i: words(p) + style_run(len(p)) for i, p in pieces.items()}}
        recovered, info = audit.logical_text(block_store(streams, reverse=True))
        self.assertEqual(recovered, units)
        self.assertEqual(info["piece_list_trailing_word"], 1)
        self.assertEqual(audit.tokenize(recovered)[0], "甲乙")

    def test_inline_footer_does_not_swallow_following_text(self):
        units = [0x1c, 1, 7, 0, 0, 3, 0x1d, ord("甲"), 0x1e, 5, 0, 1, 0x1f, ord("乙"), ord("丙")]
        text, _, info = audit.tokenize(units)
        self.assertEqual(text, "甲乙丙")
        self.assertEqual(info["plain_text_after_inline_chars"], 2)

    def test_rule_tlv_need_not_be_first(self):
        values = [100, 0, 0, 8, 3, 0x13, 94]
        payload = [0, 0x2a, 2, 0, 0, 0x8f, len(values), *values, 0xffff, 0]
        length = 3 + len(payload) + 4
        units = [0x1c, 0x10, length, *payload, length, 0, 0x10, 0x1f, ord("甲")]
        text, _, info = audit.tokenize(units)
        self.assertEqual(text, "甲")
        self.assertEqual(info["ruled_headers"], 1)
        self.assertEqual(info["ruled_headers_matching_legacy_fixed_offsets"], 0)
        self.assertEqual(info["attached_vertical_rules"], 1)
        self.assertEqual(info["invalid_rule_payloads"], 0)

    def test_style_coverage_is_independent_of_text_tokenization(self):
        event = b"\xfe\x02\x02\x01\x89\xff\0"
        result = audit.style_events(style_run(3) + event + style_run(2), 6)
        self.assertEqual(result["covered_units"], 6)
        self.assertEqual(result["change_events"], 1)
        self.assertEqual(result["end"], "substream_boundary")
        with self.assertRaises(ValueError):
            audit.style_events(style_run(7), 6)

    def test_unknown_style_does_not_hide_its_error(self):
        root = b"TextV.01" + struct.pack(">I", 1) + words([ord("甲")]) + b"\x01"
        units, info = audit.logical_text(block_store({0: root}))
        self.assertEqual(units, [ord("甲")])
        self.assertIn("error", info["styles"][0])

    def test_invalid_inline_footer_is_rejected(self):
        with self.assertRaises(ValueError):
            audit.tokenize([0x1c, 1, 7, 0, 0, 3, 0x1d, ord("甲"), 0x1e, 5, 0, 2, 0x1f])

    def test_directory_and_record_lengths_are_checked(self):
        good = block_store({0: single_piece([ord("甲")])})
        with self.assertRaises(ValueError):
            audit.logical_text(good[:-1])
        with self.assertRaises(ValueError):
            audit.tokenize([0x1c, 0x10, 99, 0, 0xffff, 0, 99, 0, 0x10, 0x1f])


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--write-probes":
        write_probes(sys.argv[2])
    else:
        unittest.main()
