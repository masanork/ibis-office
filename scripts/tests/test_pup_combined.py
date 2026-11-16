"""Fail closed when a combining sequence cannot be represented as one glyph."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pup_combined import shaped_aliases


class Font:
    def getGlyphName(self, glyph):
        return {7: "combined"}[glyph]

    def __getitem__(self, key):
        return {"combined": (1000, 0)}


class ShapingTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.table = Path(self.directory.name) / "mapping.csv"
        self.table.write_text(
            "UCSコード1,UCSコード2,後方互換PUPコード\nU+00E6,U+0300,U+10022A\n",
            encoding="utf-8",
        )
        self.addCleanup(self.directory.cleanup)

    def run_shape(self, glyphs):
        output = subprocess.CompletedProcess([], 0, stdout=json.dumps(glyphs) + "\n", stderr="")
        with patch("pup_combined.subprocess.run", return_value=output):
            return shaped_aliases("font.ttf", self.table, Font())

    def test_single_exact_glyph(self):
        glyph = {"g": 7, "dx": 0, "dy": 0, "ax": 1000, "ay": 0}
        self.assertEqual(self.run_shape([glyph]), [(0x10022A, "combined")])

    def test_rejects_multiple_missing_or_adjusted_glyphs(self):
        glyph = {"g": 7, "dx": 0, "dy": 0, "ax": 1000, "ay": 0}
        for glyphs in ([glyph, glyph], [{**glyph, "g": 0}], [{**glyph, "dx": 1}], [{**glyph, "ax": 999}]):
            with self.subTest(glyphs=glyphs), self.assertRaises(ValueError):
                self.run_shape(glyphs)

    def test_rejects_duplicate_private_use_codes(self):
        self.table.write_text(
            "UCSコード1,UCSコード2,後方互換PUPコード\nU+00E6,U+0300,U+10022A\nU+0254,U+0301,U+10022A\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(ValueError, "duplicate"):
            shaped_aliases("font.ttf", self.table, Font())


if __name__ == "__main__":
    unittest.main()
