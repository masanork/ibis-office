#!/usr/bin/env python3
"""Generate a synthetic JTTC fixture; no third-party document is included."""
import importlib.util
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('audit_tests', ROOT / 'scripts/test_audit_jtd.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
inner = (ROOT / 'test/fixtures/jtd/synthetic.jtd').read_bytes()

# A deliberately inefficient LH5 stream: one single-symbol block per byte.
bits = []
for byte in inner:
    for value, width in [(1, 16), (0, 5), (0, 5), (0, 9), (byte, 9), (0, 4), (0, 4)]:
        bits.extend((value >> shift) & 1 for shift in range(width - 1, -1, -1))
bits.extend([0] * (-len(bits) % 8))
packed = bytes(sum(bits[i + j] << (7 - j) for j in range(8)) for i in range(0, len(bits), 8))
crc = 0
for byte in inner:
    crc ^= byte
    for _ in range(8):
        crc = (crc >> 1) ^ (0xa001 if crc & 1 else 0)
archive = bytearray([22, 0]) + b'-lh5-' + struct.pack('<III', len(packed), len(inner), 0)
archive += bytes([0x20, 0, 0]) + struct.pack('<H', crc)
archive[1] = sum(archive[2:24]) & 0xff
wrapper = b'\x26\0JustCompressedDocument'.ljust(38, b'\0') + archive + packed
outer = bytearray(module.probe_cfb(wrapper))
directory = (struct.unpack_from('<I', outer, 48)[0] + 1) * 512
entry = directory + 128
name = 'JSCompDocument\0'.encode('utf-16-le')
outer[entry:entry + 64] = name.ljust(64, b'\0')
struct.pack_into('<H', outer, entry + 64, len(name))
(ROOT / 'test/fixtures/jtd/synthetic.jttc').write_bytes(outer)
