"""Writes the advance table of Inter of `text.rs` (UNITS_PER_EM, ASCII, OTHER) to stdout.

    uv run --with fonttools python text_table.py Inter-Regular.ttf

The font is `assets/fonts/Inter-Regular.ttf` of Charton 0.9.1 (Inter 4.001, git 9221beed3).
The advance of a character is the one of its glyph in `hmtx`, found through the best `cmap`.
"""

import sys

from fontTools.ttLib import TTFont

# Latin-1 and Greek, and the signs of units and numbers a label of a figure carries
OTHER = (
    list(range(0xA0, 0x100))
    + list(range(0x370, 0x400))
    + [
        0x2013,
        0x2014,
        0x2018,
        0x2019,
        0x201C,
        0x201D,
        0x2022,
        0x2026,
        0x2030,
        0x2032,
        0x2033,
    ]
    + [0x2070, 0x2074, 0x2075, 0x2076, 0x2077, 0x2078, 0x2079, 0x207A, 0x207B]
    + list(range(0x2080, 0x208A))
    + [0x2190, 0x2192, 0x2212, 0x221A, 0x221E, 0x2248, 0x2260, 0x2264, 0x2265]
)


def main(path: str) -> None:
    """Prints the glyph advance tables of the font at `path` as rust source."""
    font = TTFont(path)
    cmap = font.getBestCmap()
    hmtx = font["hmtx"]
    upem = font["head"].unitsPerEm

    def advance(code: int) -> int:
        return hmtx[cmap[code]][0]

    ascii_ = [advance(c) for c in range(0x20, 0x7F)]
    others = [c for c in OTHER if c in cmap]
    out = [
        f"/// Font units per em of the table.\npub(crate) const UNITS_PER_EM: f64 = {upem}.0;\n"
    ]
    out.append("/// Advances of U+0020 to U+007E in font units.\n#[rustfmt::skip]")
    out.append("pub(crate) const ASCII: [u16; 95] = [")
    for i in range(0, 95, 16):
        out.append("    " + ", ".join(str(a) for a in ascii_[i : i + 16]) + ",")
    out.append("];\n")
    out.append(
        "/// Advances of the other characters of the table in font units, sorted by character."
    )
    out.append(
        f"#[rustfmt::skip]\npub(crate) const OTHER: [(u32, u16); {len(others)}] = ["
    )
    for i in range(0, len(others), 6):
        out.append(
            "    " + " ".join(f"(0x{c:04x}, {advance(c)})," for c in others[i : i + 6])
        )
    out.append("];")
    print("\n".join(out))


if __name__ == "__main__":
    main(sys.argv[1])
