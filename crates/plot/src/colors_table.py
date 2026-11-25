"""Writes the colour tables of `colors_table.rs` to stdout.

    uv run --no-project --with matplotlib==3.11.2 python colors_table.py > colors_table.rs

Every table is the 256 colours of the lookup table of matplotlib's colour map of that name
(`colormaps[name](i)` for `i` in `0..256`), rounded to bytes as `matplotlib.colors.to_hex`
rounds them, so a value maps to the colour matplotlib gives it.
"""

import matplotlib
from matplotlib import colormaps
from matplotlib.colors import to_hex

# the names of the spec and of matplotlib
MAPS = [
    ("VIRIDIS", "viridis"),
    ("MAGMA", "magma"),
    ("RDBU", "RdBu"),
    ("COOLWARM", "coolwarm"),
    ("GREYS", "Greys"),
]


def main() -> None:
    """Prints the colour tables as rust source."""
    print(
        "// The colour tables of `colors.rs`, written by `colors_table.py`; do not edit."
    )
    print(f"// matplotlib {matplotlib.__version__}")
    for const, name in MAPS:
        cmap = colormaps[name]
        assert cmap.N == 256, name
        colors = [to_hex(cmap(i)) for i in range(256)]
        print()
        print(
            f"/// matplotlib's `{name}`, 256 colours `0xrrggbb` from the low end to the high end."
        )
        print(f"const {const}: [u32; 256] = [")
        for row in range(0, 256, 8):
            cells = ", ".join(f"0x{c[1:]}" for c in colors[row : row + 8])
            print(f"    {cells},")
        print("];")


if __name__ == "__main__":
    main()
