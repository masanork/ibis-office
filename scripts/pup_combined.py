"""Resolve local legacy PUP sequences to exactly shaped source-font glyphs."""
import csv
import json
import subprocess
from pathlib import Path


def shaped_aliases(font_path, table_path, font):
    """Return (PUP, glyph name) pairs; refuse ambiguous or missing glyphs."""
    rows = []
    seen = set()
    with Path(table_path).open(encoding="utf-8-sig", newline="") as stream:
        for line, row in enumerate(csv.DictReader(stream), 2):
            try:
                raw = [row[key] for key in ("UCSコード1", "UCSコード2", "後方互換PUPコード")]
                if any(not value.startswith("U+") for value in raw):
                    raise ValueError("expected U+ codepoints")
                a, b, pup = (int(value[2:], 16) for value in raw)
                sequence = chr(a) + chr(b)
                if any(0xD800 <= cp <= 0xDFFF for cp in (a, b)):
                    raise ValueError("invalid Unicode scalar")
                if any(ch in sequence for ch in "\r\n"):
                    raise ValueError("line break in sequence")
                if not 0x100000 <= pup <= 0x10FFFD:
                    raise ValueError("not a Plane 16 PUP")
                if pup in seen:
                    raise ValueError("duplicate PUP")
            except (KeyError, ValueError, OverflowError) as err:
                raise ValueError(f"invalid combining table row {line}: {err}") from err
            seen.add(pup)
            rows.append((pup, sequence))
    if not rows:
        raise ValueError("empty combining table")
    result = subprocess.run(
        ["hb-shape", "--output-format=json", "--no-glyph-names", str(font_path)],
        input="".join(sequence + "\n" for _, sequence in rows),
        text=True, capture_output=True, check=True,
    )
    shaped = [json.loads(line) for line in result.stdout.splitlines()]
    if len(shaped) != len(rows):
        raise ValueError("shaper returned a different number of lines")
    aliases = []
    for (pup, _), glyphs in zip(rows, shaped):
        if len(glyphs) != 1 or not glyphs[0]["g"]:
            raise ValueError(f"U+{pup:X}: expected one nonzero glyph")
        glyph = glyphs[0]
        name = font.getGlyphName(glyph["g"])
        # A simple cmap alias is equivalent only when shaping adds no offset
        # or advance adjustment. Otherwise a composite layout is required.
        if any(glyph[key] for key in ("dx", "dy", "ay")) or glyph["ax"] != font["hmtx"][name][0]:
            raise ValueError(f"U+{pup:X}: shaped positioning cannot be represented by an alias")
        aliases.append((pup, name))
    return aliases
