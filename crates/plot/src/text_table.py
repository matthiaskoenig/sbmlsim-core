"""Writes the advance tables of Inter of `text.rs` to stdout.

    uv run --no-project --with fonttools python text_table.py Inter-Regular.ttf Inter-Bold.ttf \
        NotoSansMath-Regular.ttf

The tables are UNITS_PER_EM, ASCII and OTHER of the regular font and, since 0.28.0,
ASCII_BOLD and OTHER_BOLD of the bold one. The regular font is `assets/fonts/Inter-Regular.ttf`
of Charton 0.9.1 (Inter 4.001, git 9221beed3), the bold one `crates/plot/fonts/Inter-Bold.ttf`
of the same version. The advance of a character is the one of its glyph in `hmtx`, found
through the best `cmap`. Since 0.31.1 the tables also have the arrows and the mathematical
operators Inter has not (the dot operator U+22C5 of a unit), with the advance of the font the PNG
draws them with, the subset of Noto Sans Math of `crates/plot/fonts`, scaled to the units of
Inter; the bold tables take them from the same font.
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
    + list(range(0x2190, 0x2300))
)


def tables(path: str, fallback: str, suffix: str) -> tuple[int, list[str]]:
    """The units per em of the font at `path` and its advance tables as rust source.

    A character the font has not takes its advance in the font at `fallback`.
    """
    font = TTFont(path)
    cmap = font.getBestCmap()
    hmtx = font["hmtx"]
    upem = font["head"].unitsPerEm
    other = TTFont(fallback)
    other_cmap = other.getBestCmap()
    other_hmtx = other["hmtx"]
    scale = upem / other["head"].unitsPerEm

    def advance(code: int) -> int:
        if code in cmap:
            return hmtx[cmap[code]][0]
        return round(other_hmtx[other_cmap[code]][0] * scale)

    ascii_ = [advance(c) for c in range(0x20, 0x7F)]
    others = [c for c in OTHER if c in cmap or c in other_cmap]
    weight = " in the bold font" if suffix else ""
    out = [f"/// Advances of U+0020 to U+007E{weight} in font units.\n#[rustfmt::skip]"]
    out.append(f"pub(crate) const ASCII{suffix}: [u16; 95] = [")
    for i in range(0, 95, 16):
        out.append("    " + ", ".join(str(a) for a in ascii_[i : i + 16]) + ",")
    out.append("];\n")
    out.append(
        f"/// Advances of the other characters of the table{weight} in font units, sorted by"
        " character."
    )
    out.append(
        f"#[rustfmt::skip]\npub(crate) const OTHER{suffix}: [(u32, u16); {len(others)}] = ["
    )
    for i in range(0, len(others), 6):
        out.append(
            "    " + " ".join(f"(0x{c:04x}, {advance(c)})," for c in others[i : i + 6])
        )
    out.append("];")
    return upem, out


def main(regular: str, bold: str, fallback: str) -> None:
    """Prints the glyph advance tables of the regular and the bold font as rust source."""
    upem, out = tables(regular, fallback, "")
    bold_upem, bold_out = tables(bold, fallback, "_BOLD")
    if bold_upem != upem:
        raise SystemExit(f"units per em differ: {upem} and {bold_upem}")
    head = f"/// Font units per em of the table.\npub(crate) const UNITS_PER_EM: f64 = {upem}.0;\n"
    print("\n".join([head, *out, "", *bold_out]))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3])
