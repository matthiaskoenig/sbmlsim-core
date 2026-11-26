"""Writes the subsets of the Noto fonts the PNG embeds besides Inter (since 0.31.0).

    uv run --no-project --with fonttools python crates/plot/fonts/subset.py \
        NotoSansMono-Regular.ttf NotoSansMono-Bold.ttf NotoSansMath-Regular.ttf crates/plot/fonts

`NotoSansMono-Regular.ttf` and `NotoSansMono-Bold.ttf` (Noto Sans Mono 2.006) draw a text of the
family `monospace`, every character 600 of 1000 units wide; its subset keeps Latin-1, Greek, the punctuation, the super-
and subscripts, the arrows and the mathematical operators. `NotoSansMath-Regular.ttf` (Noto Sans
Math 2.001) draws the arrows and the mathematical operators Inter has not, e.g. the dot operator
U+22C5 of a unit, which the PNG takes from it as its fallback. Both fonts are under the SIL Open
Font License 1.1, as Inter.
"""

import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

MONO = [
    *range(0x20, 0x7F),
    *range(0xA0, 0x100),
    *range(0x370, 0x400),
    *range(0x2010, 0x205F),
    *range(0x2070, 0x20A0),
    *range(0x2190, 0x2200),
    *range(0x2200, 0x2300),
]
MATH = [*range(0x2190, 0x2200), *range(0x2200, 0x2300)]


def write(source: str, codes: list[int], target: Path) -> None:
    """Writes the subset of the characters `codes` of the font at `source` to `target`."""
    font = TTFont(source)
    options = subset.Options()
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.notdef_outline = True
    options.glyph_names = False
    subsetter = subset.Subsetter(options)
    cmap = font.getBestCmap()
    subsetter.populate(unicodes=[c for c in codes if c in cmap])
    subsetter.subset(font)
    font.save(target)


def main(mono: str, mono_bold: str, math: str, out: str) -> None:
    """Writes the subsets of Noto Sans Mono and Noto Sans Math into the directory `out`."""
    write(mono, MONO, Path(out) / "NotoSansMono-Regular.ttf")
    write(mono_bold, MONO, Path(out) / "NotoSansMono-Bold.ttf")
    write(math, MATH, Path(out) / "NotoSansMath-Regular.ttf")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4])
