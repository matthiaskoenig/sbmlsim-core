"""Writes the colour tables of `colors_table.rs` to stdout.

    uv run --no-project --with matplotlib==3.11.2 python colors_table.py > colors_table.rs

Every table is the 256 colours of the lookup table of matplotlib's colour map of that name
(`colormaps[name](i)` for `i` in `0..256`), rounded to bytes as `matplotlib.colors.to_hex`
rounds them, so a value maps to the colour matplotlib gives it. The maps are the sequential
and the diverging maps of matplotlib (its groups "Perceptually Uniform Sequential",
"Sequential", "Sequential (2)" and "Diverging"); every map is named in lower case, as the
spec names it, and has a reversed variant `<name>_r`, which reads its table from the high end.
"""

import matplotlib
from matplotlib import colormaps
from matplotlib.colors import to_hex

# the maps of matplotlib by its names, in the order of its groups
MAPS = [
    # perceptually uniform sequential
    "viridis",
    "plasma",
    "inferno",
    "magma",
    "cividis",
    # sequential
    "Greys",
    "Purples",
    "Blues",
    "Greens",
    "Oranges",
    "Reds",
    "YlOrBr",
    "YlOrRd",
    "OrRd",
    "PuRd",
    "RdPu",
    "BuPu",
    "GnBu",
    "PuBu",
    "YlGnBu",
    "PuBuGn",
    "BuGn",
    "YlGn",
    # sequential (2)
    "binary",
    "gist_yarg",
    "gist_gray",
    "gray",
    "bone",
    "pink",
    "spring",
    "summer",
    "autumn",
    "winter",
    "cool",
    "Wistia",
    "hot",
    "afmhot",
    "gist_heat",
    "copper",
    # diverging
    "PiYG",
    "PRGn",
    "BrBG",
    "PuOr",
    "RdGy",
    "RdBu",
    "RdYlBu",
    "RdYlGn",
    "Spectral",
    "coolwarm",
    "bwr",
    "seismic",
    "berlin",
    "managua",
    "vanimo",
]


def variant(name: str) -> str:
    """The name of the variant of the rust enum of a spec name, e.g. `GistYargR`."""
    return "".join(part.capitalize() for part in name.split("_"))


def lut(name: str) -> list[str]:
    """The 256 colours `#rrggbb` of a colour map of matplotlib."""
    cmap = colormaps[name]
    assert cmap.N == 256, name
    return [to_hex(cmap(i)) for i in range(256)]


def main() -> None:
    """Prints the colour tables, the enum of their names and its methods as rust source."""
    names = [m.lower() for m in MAPS]
    assert len(set(names)) == len(names)
    # a reversed map is its table read from the high end, but where matplotlib's `<name>_r`
    # rounds a colour of a map of segments otherwise, which then has a table of its own
    own = [m for m in MAPS if lut(m)[::-1] != lut(f"{m}_r")]
    tables = MAPS + [f"{m}_r" for m in own]
    print(
        "// The colour tables of `colors.rs`, written by `colors_table.py`; do not edit."
    )
    print(f"// matplotlib {matplotlib.__version__}")
    print()
    print(
        "/// The number of tables: one per colour map, and one per reversed map whose colours"
    )
    print("/// are not its map read from the high end.")
    print(f"const TABLE_COUNT: usize = {len(tables)};")
    print()
    print(
        "/// The tables of the colour maps, 256 colours `0xrrggbb` each from the low end to the"
    )
    print("/// high end.")
    print("#[rustfmt::skip]")
    print("static TABLES: [[u32; 256]; TABLE_COUNT] = [")
    for m in tables:
        colors = lut(m)
        print(f"    // {m}")
        print("    [")
        for row in range(0, 256, 8):
            cells = ", ".join(f"0x{c[1:]}" for c in colors[row : row + 8])
            print(f"        {cells},")
        print("    ],")
    print("];")
    print()
    print(
        "/// The names of the colour scales, the values of `name` of a scale of the spec: every"
    )
    print("/// map followed by its reversed variant.")
    print("pub const COLOR_SCALES: &[&str] = &[")
    for n in names:
        print(f'    "{n}",')
        print(f'    "{n}_r",')
    print("];")
    print()
    print(
        "/// The name of a colour map of a colour scale: a sequential or diverging colour map of"
    )
    print("/// matplotlib in lower case, or its reversed variant `<name>_r`.")
    print("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]")
    print("pub enum ColorScaleName {")
    for m, n in zip(MAPS, names, strict=True):
        print(f"    /// matplotlib's `{m}`.")
        print(f"    {variant(n)},")
        print(f"    /// matplotlib's `{m}_r`, `{n}` reversed.")
        print(f"    {variant(n + '_r')},")
    print("}")
    print()
    print("impl ColorScaleName {")
    print("    /// Every colour scale name, each map followed by its reversed variant.")
    print(f"    pub const ALL: [ColorScaleName; {2 * len(MAPS)}] = [")
    for n in names:
        print(f"        ColorScaleName::{variant(n)},")
        print(f"        ColorScaleName::{variant(n + '_r')},")
    print("    ];")
    print()
    print("    /// The name of the scale in the spec, e.g. `blues` or `rdbu_r`.")
    print("    pub const fn name(self) -> &'static str {")
    print("        match self {")
    for n in names:
        print(f'            ColorScaleName::{variant(n)} => "{n}",')
        print(f'            ColorScaleName::{variant(n + "_r")} => "{n}_r",')
    print("        }")
    print("    }")
    print()
    print(
        "    /// The table of the scale and whether the scale reads it from the high end."
    )
    print("    fn table(self) -> (&'static [u32; 256], bool) {")
    print("        let (map, reversed) = match self {")
    for k, (m, n) in enumerate(zip(MAPS, names, strict=True)):
        print(f"            ColorScaleName::{variant(n)} => ({k}, false),")
        if m in own:
            k_r = tables.index(f"{m}_r")
            print(f"            ColorScaleName::{variant(n + '_r')} => ({k_r}, false),")
        else:
            print(f"            ColorScaleName::{variant(n + '_r')} => ({k}, true),")
    print("        };")
    print("        (&TABLES[map], reversed)")
    print("    }")
    print("}")


if __name__ == "__main__":
    main()
