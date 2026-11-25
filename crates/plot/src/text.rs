//! The width of a text in the bundled font, Inter, from a table of its advances.
//!
//! Every width the crate lays out with is measured here, so a figure is laid out the same
//! natively and in wasm, whatever fonts the machine has. The table is the advance of every
//! character of ASCII, Latin-1, Greek and of a few signs of units and numbers in
//! `Inter-Regular.ttf` 4.001 (git 9221beed3), the font Charton 0.9.1 bundles
//! (`assets/fonts/Inter-Regular.ttf`). It was written by `text_table.py` next to this file,
//! with fontTools, outside of the uv project of the repository:
//!
//! ```text
//! uv run --no-project --with fonttools python crates/plot/src/text_table.py Inter-Regular.ttf
//! ```
//!
//! which takes `head.unitsPerEm` and, for every character of the ranges, the
//! advance of its glyph in `hmtx` through the best `cmap`. Kerning is not applied; a character
//! outside of the table takes the advance of the digit zero.

/// Font units per em of the table.
pub(crate) const UNITS_PER_EM: f64 = 2048.0;

/// Advances of U+0020 to U+007E in font units.
#[rustfmt::skip]
pub(crate) const ASCII: [u16; 95] = [
    576, 589, 954, 1297, 1314, 2011, 1319, 614, 747, 747, 1026, 1355, 590, 942, 590, 738,
    1292, 833, 1249, 1265, 1323, 1215, 1270, 1159, 1267, 1270, 590, 618, 1355, 1355, 1355, 1047,
    1978, 1413, 1340, 1496, 1478, 1231, 1209, 1528, 1522, 550, 1169, 1376, 1158, 1850, 1543, 1566,
    1308, 1566, 1318, 1314, 1322, 1524, 1413, 2018, 1397, 1390, 1288, 747, 738, 747, 965, 934,
    661, 1150, 1254, 1170, 1254, 1194, 758, 1256, 1211, 496, 496, 1124, 496, 1794, 1210, 1228,
    1254, 1254, 771, 1081, 670, 1211, 1151, 1676, 1118, 1151, 1131, 873, 681, 873, 1355,
];

/// Advances of the other characters of the table in font units, sorted by character.
#[rustfmt::skip]
pub(crate) const OTHER: [(u32, u16); 239] = [
    (0x00a0, 576), (0x00a1, 589), (0x00a2, 1170), (0x00a3, 1251), (0x00a4, 1484), (0x00a5, 1126),
    (0x00a6, 553), (0x00a7, 1164), (0x00a8, 1210), (0x00a9, 1872), (0x00aa, 929), (0x00ab, 1193),
    (0x00ac, 1355), (0x00ae, 1364), (0x00af, 978), (0x00b0, 933), (0x00b1, 1355), (0x00b2, 905),
    (0x00b3, 913), (0x00b4, 661), (0x00b5, 1200), (0x00b6, 1234), (0x00b7, 590), (0x00b8, 549),
    (0x00b9, 623), (0x00ba, 988), (0x00bb, 1193), (0x00bc, 1643), (0x00bd, 1735), (0x00be, 1806),
    (0x00bf, 1047), (0x00c0, 1413), (0x00c1, 1413), (0x00c2, 1413), (0x00c3, 1413), (0x00c4, 1413),
    (0x00c5, 1413), (0x00c6, 2035), (0x00c7, 1496), (0x00c8, 1231), (0x00c9, 1231), (0x00ca, 1231),
    (0x00cb, 1231), (0x00cc, 550), (0x00cd, 550), (0x00ce, 550), (0x00cf, 550), (0x00d0, 1505),
    (0x00d1, 1543), (0x00d2, 1566), (0x00d3, 1566), (0x00d4, 1566), (0x00d5, 1566), (0x00d6, 1566),
    (0x00d7, 1355), (0x00d8, 1566), (0x00d9, 1524), (0x00da, 1524), (0x00db, 1524), (0x00dc, 1524),
    (0x00dd, 1390), (0x00de, 1302), (0x00df, 1262), (0x00e0, 1150), (0x00e1, 1150), (0x00e2, 1150),
    (0x00e3, 1150), (0x00e4, 1150), (0x00e5, 1150), (0x00e6, 1879), (0x00e7, 1170), (0x00e8, 1194),
    (0x00e9, 1194), (0x00ea, 1194), (0x00eb, 1194), (0x00ec, 496), (0x00ed, 496), (0x00ee, 496),
    (0x00ef, 496), (0x00f0, 1193), (0x00f1, 1210), (0x00f2, 1228), (0x00f3, 1228), (0x00f4, 1228),
    (0x00f5, 1228), (0x00f6, 1228), (0x00f7, 1355), (0x00f8, 1228), (0x00f9, 1211), (0x00fa, 1211),
    (0x00fb, 1211), (0x00fc, 1211), (0x00fd, 1151), (0x00fe, 1254), (0x00ff, 1151), (0x0374, 546),
    (0x0375, 551), (0x0376, 1543), (0x037a, 549), (0x037b, 1170), (0x037c, 1170), (0x037d, 1170),
    (0x037e, 618), (0x037f, 1169), (0x0384, 553), (0x0385, 1378), (0x0386, 1413), (0x0387, 590),
    (0x0388, 1512), (0x0389, 1803), (0x038a, 831), (0x038c, 1765), (0x038e, 1799), (0x038f, 1783),
    (0x0390, 564), (0x0391, 1413), (0x0392, 1340), (0x0393, 1185), (0x0394, 1395), (0x0395, 1231),
    (0x0396, 1288), (0x0397, 1522), (0x0398, 1566), (0x0399, 550), (0x039a, 1376), (0x039b, 1413),
    (0x039c, 1850), (0x039d, 1543), (0x039e, 1344), (0x039f, 1566), (0x03a0, 1522), (0x03a1, 1308),
    (0x03a3, 1411), (0x03a4, 1322), (0x03a5, 1390), (0x03a6, 1803), (0x03a7, 1397), (0x03a8, 1802),
    (0x03a9, 1577), (0x03aa, 550), (0x03ab, 1390), (0x03ac, 1360), (0x03ad, 1100), (0x03ae, 1211),
    (0x03af, 564), (0x03b0, 1211), (0x03b1, 1360), (0x03b2, 1220), (0x03b3, 1151), (0x03b4, 1196),
    (0x03b5, 1100), (0x03b6, 1043), (0x03b7, 1211), (0x03b8, 1272), (0x03b9, 564), (0x03ba, 1076),
    (0x03bb, 1314), (0x03bc, 1200), (0x03bd, 1202), (0x03be, 1144), (0x03bf, 1228), (0x03c0, 1239),
    (0x03c1, 1254), (0x03c2, 1166), (0x03c3, 1278), (0x03c4, 954), (0x03c5, 1211), (0x03c6, 1476),
    (0x03c7, 1165), (0x03c8, 1593), (0x03c9, 1648), (0x03ca, 564), (0x03cb, 1211), (0x03cc, 1228),
    (0x03cd, 1211), (0x03ce, 1648), (0x03cf, 1376), (0x03d0, 1298), (0x03d1, 1300), (0x03d2, 1375),
    (0x03d3, 1730), (0x03d4, 1375), (0x03d5, 1403), (0x03d6, 1680), (0x03d7, 1076), (0x03dc, 1215),
    (0x03dd, 1093), (0x03f0, 1484), (0x03f1, 1298), (0x03f2, 1170), (0x03f3, 496), (0x03f4, 1566),
    (0x03f5, 922), (0x03f6, 922), (0x03f9, 1496), (0x03fa, 1850), (0x03fc, 1254), (0x03fd, 1496),
    (0x03fe, 1496), (0x03ff, 1496), (0x2013, 1024), (0x2014, 2048), (0x2018, 534), (0x2019, 534),
    (0x201c, 902), (0x201d, 902), (0x2022, 1152), (0x2026, 1770), (0x2030, 2735), (0x2032, 456),
    (0x2033, 906), (0x2070, 941), (0x2074, 942), (0x2075, 901), (0x2076, 909), (0x2077, 831),
    (0x2078, 922), (0x2079, 909), (0x207a, 878), (0x207b, 879), (0x2080, 941), (0x2081, 623),
    (0x2082, 905), (0x2083, 913), (0x2084, 942), (0x2085, 901), (0x2086, 909), (0x2087, 831),
    (0x2088, 922), (0x2089, 909), (0x2190, 1954), (0x2192, 1954), (0x2212, 1355), (0x221a, 1289),
    (0x221e, 1917), (0x2248, 1355), (0x2260, 1355), (0x2264, 1355), (0x2265, 1355),
];

/// The advance of a character in font units.
fn advance(c: char) -> u16 {
    let code = c as u32;
    if (0x20..0x7f).contains(&code) {
        return ASCII[(code - 0x20) as usize];
    }
    match OTHER.binary_search_by_key(&code, |&(c, _)| c) {
        Ok(i) => OTHER[i].1,
        Err(_) => ASCII[(u32::from('0') - 0x20) as usize],
    }
}

/// The width of a text in pixels at a font size in pixels, in Inter Regular.
pub fn text_width(text: &str, size: f64) -> f64 {
    let units: u64 = text.chars().map(|c| u64::from(advance(c))).sum();
    units as f64 * size / UNITS_PER_EM
}

/// Charton's estimate of the width of a text (`estimate_text_width` of `core/utils.rs` of
/// Charton 0.9.1, which is not public): what Charton reserves for a tick label, which the crate
/// corrects to [`text_width`].
pub(crate) fn charton_estimate(text: &str, size: f64) -> f64 {
    let (mut wide, mut narrow, mut upper, mut other) = (0u32, 0u32, 0u32, 0u32);
    for c in text.chars() {
        if is_wide(c) {
            wide += 1;
        } else if matches!(
            c,
            '.' | ',' | ':' | ';' | '!' | 'i' | 'j' | 'l' | '-' | '|' | '1' | 't' | 'f' | 'r'
        ) {
            narrow += 1;
        } else if c.is_ascii_uppercase() {
            upper += 1;
        } else {
            other += 1;
        }
    }
    (f64::from(wide) + f64::from(narrow) * 0.3 + f64::from(upper) * 0.65 + f64::from(other) * 0.55)
        * size
}

/// Whether Charton counts a character one em wide (`is_wide` of Charton 0.9.1).
const fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE10..=0xFE19
        | 0xFE30..=0xFE4F
        | 0xFF01..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F9FF
        | 0x20000..=0x3FFFD
    )
}

/// The extent across its axis of a tick label of a width, rotated by `angle` degrees: the side
/// of its rotated line box (the line is one em, the font size) which the axis band reserves, as
/// Charton computes it, with the sine and cosine of `libm`.
pub(crate) fn label_extent(width: f64, size: f64, angle: f64, bottom: bool) -> f64 {
    let radians = angle * std::f64::consts::PI / 180.0;
    let (sin, cos) = if angle == 0.0 {
        (0.0, 1.0)
    } else {
        (libm::sin(radians).abs(), libm::cos(radians).abs())
    };
    if bottom {
        width * sin + size * cos
    } else {
        width * cos + size * sin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_of_the_spike() {
        // FINDINGS.md of the spike, fontTools at 13 px
        for (text, width) in [
            ("0", 8.2),
            ("10", 13.5),
            ("1000", 29.9),
            ("Smith2001", 64.4),
            ("concentration [mg/l]", 125.2),
        ] {
            assert!((text_width(text, 13.0) - width).abs() < 0.06, "{text}");
        }
    }

    #[test]
    fn estimate_of_charton() {
        // the class counts of Charton: 1 narrow, 0 0.55 each, at 13 px
        assert!((charton_estimate("10", 13.0) - 0.85 * 13.0).abs() < 1e-12);
        assert_eq!(label_extent(20.0, 11.0, 0.0, true), 11.0);
        assert_eq!(label_extent(20.0, 11.0, 0.0, false), 20.0);
        assert!((label_extent(20.0, 11.0, 90.0, true) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn table_is_sorted() {
        assert!(OTHER.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(text_width("µ°²αβ", 10.0) > 0.0);
    }
}
