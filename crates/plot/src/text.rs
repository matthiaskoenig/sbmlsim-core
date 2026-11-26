//! The width of a text in the bundled font, Inter, from a table of its advances.
//!
//! Every width the crate lays out with is measured here, so a figure is laid out the same
//! natively and in wasm, whatever fonts the machine has. The tables are the advance of every
//! character of ASCII, Latin-1, Greek and of a few signs of units and numbers in
//! `Inter-Regular.ttf` 4.001 (git 9221beed3), the font Charton 0.9.1 bundles
//! (`assets/fonts/Inter-Regular.ttf`), and, since 0.28.0, in `Inter-Bold.ttf` of the same
//! version (`crates/plot/fonts/Inter-Bold.ttf`), which measures a text of the weight bold. They
//! were written by `text_table.py` next to this file, with fontTools, outside of the uv project
//! of the repository:
//!
//! ```text
//! uv run --no-project --with fonttools python crates/plot/src/text_table.py \
//!     crates/plot/fonts/Inter-Regular.ttf crates/plot/fonts/Inter-Bold.ttf
//! ```
//!
//! which takes `head.unitsPerEm` and, for every character of the ranges, the
//! advance of its glyph in `hmtx` through the best `cmap`. Kerning is not applied; a character
//! outside of the table takes the advance of the digit zero.
//!
//! A text of several lines (since 0.28.0) is split at every `\n`: it is as wide as its widest
//! line, and its lines are [`LINE_HEIGHT`] font sizes apart.

use crate::spec::{FontFamily, FontWeight};

/// The distance of two lines of a text in font sizes.
pub(crate) const LINE_HEIGHT: f64 = 1.2;

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
pub(crate) const OTHER: [(u32, u16); 587] = [
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
    (0x2088, 922), (0x2089, 909), (0x2190, 1954), (0x2191, 1708), (0x2192, 1954), (0x2193, 1708),
    (0x2194, 2746), (0x2195, 1708), (0x2196, 1588), (0x2197, 1588), (0x2198, 1588), (0x2199, 1588),
    (0x219a, 1626), (0x219b, 1626), (0x219c, 1749), (0x219d, 1749), (0x219e, 1831), (0x219f, 893),
    (0x21a0, 1831), (0x21a1, 893), (0x21a2, 1642), (0x21a3, 1642), (0x21a4, 1626), (0x21a5, 893),
    (0x21a6, 1626), (0x21a7, 893), (0x21a8, 893), (0x21a9, 2343), (0x21aa, 2343), (0x21ab, 1761),
    (0x21ac, 1761), (0x21ad, 1952), (0x21ae, 1810), (0x21b0, 2022), (0x21b1, 2022), (0x21b2, 1237),
    (0x21b3, 2022), (0x21b4, 1709), (0x21b5, 2022), (0x21b6, 2091), (0x21b7, 2091), (0x21b8, 1692),
    (0x21b9, 2032), (0x21ba, 1954), (0x21bb, 1954), (0x21bc, 1626), (0x21bd, 1626), (0x21be, 893),
    (0x21bf, 893), (0x21c0, 1626), (0x21c1, 1626), (0x21c2, 893), (0x21c3, 893), (0x21c4, 1626),
    (0x21c5, 1565), (0x21c6, 1626), (0x21c7, 1626), (0x21c8, 1565), (0x21c9, 1626), (0x21ca, 1565),
    (0x21cb, 1626), (0x21cc, 1626), (0x21cd, 1647), (0x21ce, 2015), (0x21cf, 1647), (0x21d0, 1954),
    (0x21d1, 1257), (0x21d2, 1954), (0x21d3, 1257), (0x21d4, 2746), (0x21d5, 1257), (0x21d6, 1384),
    (0x21d7, 1384), (0x21d8, 1384), (0x21d9, 1384), (0x21da, 1647), (0x21db, 1647), (0x21dc, 1774),
    (0x21dd, 1774), (0x21de, 1245), (0x21df, 1245), (0x21e0, 1710), (0x21e1, 893), (0x21e2, 1710),
    (0x21e3, 893), (0x21e4, 2078), (0x21e5, 2078), (0x21e7, 2111), (0x21ea, 2111), (0x21f1, 1837),
    (0x21f2, 1837), (0x21f4, 1731), (0x21f5, 1565), (0x21f6, 1626), (0x21f7, 1626), (0x21f8, 1626),
    (0x21f9, 1810), (0x21fa, 1626), (0x21fb, 1626), (0x21fc, 1810), (0x21fd, 1753), (0x21fe, 1753),
    (0x21ff, 2060), (0x2200, 1536), (0x2201, 883), (0x2202, 1298), (0x2203, 1176), (0x2204, 1176),
    (0x2205, 1402), (0x2206, 1395), (0x2207, 1190), (0x2208, 1432), (0x2209, 1432), (0x220a, 1139),
    (0x220b, 1432), (0x220c, 1432), (0x220d, 1139), (0x220e, 1143), (0x220f, 1480), (0x2210, 1329),
    (0x2211, 1299), (0x2212, 1355), (0x2213, 1171), (0x2214, 1171), (0x2215, 825), (0x2216, 825),
    (0x2217, 934), (0x2218, 782), (0x2219, 594), (0x221a, 1289), (0x221b, 1239), (0x221c, 1239),
    (0x221d, 1106), (0x221e, 1917), (0x221f, 1335), (0x2220, 1335), (0x2221, 1335), (0x2222, 1171),
    (0x2223, 354), (0x2224, 883), (0x2225, 737), (0x2226, 1262), (0x2227, 1462), (0x2228, 1462),
    (0x2229, 1401), (0x222a, 1401), (0x222b, 748), (0x222c, 1323), (0x222d, 1853), (0x222e, 1116),
    (0x222f, 1683), (0x2230, 2275), (0x2231, 1200), (0x2232, 1198), (0x2233, 1227), (0x2234, 1169),
    (0x2235, 1171), (0x2236, 590), (0x2237, 1171), (0x2238, 1171), (0x2239, 1264), (0x223a, 1171),
    (0x223b, 1171), (0x223c, 1171), (0x223d, 1171), (0x223e, 1425), (0x223f, 1366), (0x2240, 498),
    (0x2241, 1171), (0x2242, 1171), (0x2243, 1171), (0x2244, 1171), (0x2245, 1171), (0x2246, 1171),
    (0x2247, 1171), (0x2248, 1355), (0x2249, 1171), (0x224a, 1171), (0x224b, 1171), (0x224c, 1171),
    (0x224d, 1171), (0x224e, 1171), (0x224f, 1171), (0x2250, 1171), (0x2251, 1171), (0x2252, 1171),
    (0x2253, 1171), (0x2254, 1264), (0x2255, 1264), (0x2256, 1171), (0x2257, 1171), (0x2258, 1171),
    (0x2259, 1171), (0x225a, 1171), (0x225b, 1171), (0x225c, 1171), (0x225d, 1171), (0x225e, 1171),
    (0x225f, 1171), (0x2260, 1355), (0x2261, 1171), (0x2262, 1171), (0x2263, 1171), (0x2264, 1355),
    (0x2265, 1355), (0x2266, 1171), (0x2267, 1171), (0x2268, 1171), (0x2269, 1171), (0x226a, 1706),
    (0x226b, 1706), (0x226c, 713), (0x226d, 1171), (0x226e, 1171), (0x226f, 1171), (0x2270, 1171),
    (0x2271, 1171), (0x2272, 1171), (0x2273, 1171), (0x2274, 1171), (0x2275, 1171), (0x2276, 1171),
    (0x2277, 1171), (0x2278, 1171), (0x2279, 1171), (0x227a, 1110), (0x227b, 1110), (0x227c, 1110),
    (0x227d, 1110), (0x227e, 1171), (0x227f, 1171), (0x2280, 1110), (0x2281, 1110), (0x2282, 1432),
    (0x2283, 1432), (0x2284, 1432), (0x2285, 1432), (0x2286, 1432), (0x2287, 1432), (0x2288, 1432),
    (0x2289, 1432), (0x228a, 1432), (0x228b, 1432), (0x228c, 1319), (0x228d, 1319), (0x228e, 1319),
    (0x228f, 1171), (0x2290, 1319), (0x2291, 1319), (0x2292, 1319), (0x2293, 1319), (0x2294, 1319),
    (0x2295, 2846), (0x2296, 2846), (0x2297, 2846), (0x2298, 2846), (0x2299, 1518), (0x229a, 1901),
    (0x229b, 1901), (0x229c, 1901), (0x229d, 1901), (0x229e, 1548), (0x229f, 1548), (0x22a0, 1548),
    (0x22a1, 1548), (0x22a2, 1548), (0x22a3, 1548), (0x22a4, 1548), (0x22a5, 1548), (0x22a6, 952),
    (0x22a7, 961), (0x22a8, 1548), (0x22a9, 1712), (0x22aa, 1731), (0x22ab, 1712), (0x22ac, 1548),
    (0x22ad, 1548), (0x22ae, 1712), (0x22af, 1712), (0x22b0, 1161), (0x22b1, 1161), (0x22b2, 1171),
    (0x22b3, 1171), (0x22b4, 1171), (0x22b5, 1171), (0x22b6, 1872), (0x22b7, 1872), (0x22b8, 1630),
    (0x22b9, 1171), (0x22ba, 1139), (0x22bb, 1171), (0x22bc, 1171), (0x22bd, 1171), (0x22be, 1335),
    (0x22bf, 1440), (0x22c0, 1884), (0x22c1, 1884), (0x22c2, 1577), (0x22c3, 1577), (0x22c4, 1206),
    (0x22c5, 455), (0x22c6, 971), (0x22c7, 1171), (0x22c8, 1503), (0x22c9, 1503), (0x22ca, 1503),
    (0x22cb, 1507), (0x22cc, 1507), (0x22cd, 1171), (0x22ce, 1495), (0x22cf, 1495), (0x22d0, 1432),
    (0x22d1, 1432), (0x22d2, 1401), (0x22d3, 1401), (0x22d4, 1397), (0x22d5, 1229), (0x22d6, 1090),
    (0x22d7, 1090), (0x22d8, 2357), (0x22d9, 2357), (0x22da, 1171), (0x22db, 1171), (0x22dc, 1171),
    (0x22dd, 1171), (0x22de, 1110), (0x22df, 1110), (0x22e0, 1110), (0x22e1, 1110), (0x22e2, 1171),
    (0x22e3, 1171), (0x22e4, 1171), (0x22e5, 1171), (0x22e6, 1171), (0x22e7, 1171), (0x22e8, 1171),
    (0x22e9, 1171), (0x22ea, 1171), (0x22eb, 1171), (0x22ec, 1171), (0x22ed, 1171), (0x22ee, 455),
    (0x22ef, 1700), (0x22f0, 1712), (0x22f1, 1712), (0x22f2, 1671), (0x22f3, 1432), (0x22f4, 1139),
    (0x22f5, 1432), (0x22f6, 1432), (0x22f7, 1139), (0x22f8, 1432), (0x22f9, 1432), (0x22fa, 1671),
    (0x22fb, 1432), (0x22fc, 1139), (0x22fd, 1432), (0x22fe, 1139), (0x22ff, 1411),
];

/// Advances of U+0020 to U+007E in the bold font in font units.
#[rustfmt::skip]
pub(crate) const ASCII_BOLD: [u16; 95] = [
    485, 692, 1130, 1329, 1341, 2080, 1376, 694, 772, 772, 1145, 1390, 684, 958, 684, 795,
    1381, 883, 1290, 1322, 1385, 1274, 1330, 1191, 1333, 1330, 684, 702, 1390, 1390, 1390, 1146,
    2081, 1529, 1355, 1515, 1479, 1244, 1202, 1537, 1530, 575, 1197, 1473, 1158, 1908, 1561, 1578,
    1327, 1591, 1345, 1341, 1367, 1499, 1529, 2125, 1512, 1497, 1360, 772, 795, 772, 997, 975,
    748, 1189, 1291, 1205, 1291, 1220, 815, 1294, 1275, 555, 555, 1188, 555, 1869, 1275, 1256,
    1291, 1291, 834, 1147, 750, 1275, 1228, 1741, 1188, 1233, 1173, 960, 761, 960, 1390,
];

/// Advances of the other characters of the table in the bold font in font units, sorted by character.
#[rustfmt::skip]
pub(crate) const OTHER_BOLD: [(u32, u16); 587] = [
    (0x00a0, 485), (0x00a1, 692), (0x00a2, 1205), (0x00a3, 1308), (0x00a4, 1560), (0x00a5, 1168),
    (0x00a6, 694), (0x00a7, 1164), (0x00a8, 1275), (0x00a9, 1872), (0x00aa, 950), (0x00ab, 1352),
    (0x00ac, 1390), (0x00ae, 1356), (0x00af, 899), (0x00b0, 941), (0x00b1, 1390), (0x00b2, 942),
    (0x00b3, 966), (0x00b4, 748), (0x00b5, 1286), (0x00b6, 1226), (0x00b7, 684), (0x00b8, 746),
    (0x00b9, 682), (0x00ba, 1003), (0x00bb, 1352), (0x00bc, 1732), (0x00bd, 1805), (0x00be, 1884),
    (0x00bf, 1146), (0x00c0, 1529), (0x00c1, 1529), (0x00c2, 1529), (0x00c3, 1529), (0x00c4, 1529),
    (0x00c5, 1529), (0x00c6, 2094), (0x00c7, 1515), (0x00c8, 1244), (0x00c9, 1244), (0x00ca, 1244),
    (0x00cb, 1244), (0x00cc, 575), (0x00cd, 575), (0x00ce, 575), (0x00cf, 575), (0x00d0, 1556),
    (0x00d1, 1561), (0x00d2, 1578), (0x00d3, 1578), (0x00d4, 1578), (0x00d5, 1578), (0x00d6, 1578),
    (0x00d7, 1390), (0x00d8, 1578), (0x00d9, 1499), (0x00da, 1499), (0x00db, 1499), (0x00dc, 1499),
    (0x00dd, 1497), (0x00de, 1370), (0x00df, 1346), (0x00e0, 1189), (0x00e1, 1189), (0x00e2, 1189),
    (0x00e3, 1189), (0x00e4, 1189), (0x00e5, 1189), (0x00e6, 1864), (0x00e7, 1205), (0x00e8, 1220),
    (0x00e9, 1220), (0x00ea, 1220), (0x00eb, 1220), (0x00ec, 555), (0x00ed, 555), (0x00ee, 555),
    (0x00ef, 555), (0x00f0, 1227), (0x00f1, 1275), (0x00f2, 1256), (0x00f3, 1256), (0x00f4, 1256),
    (0x00f5, 1256), (0x00f6, 1256), (0x00f7, 1390), (0x00f8, 1256), (0x00f9, 1275), (0x00fa, 1275),
    (0x00fb, 1275), (0x00fc, 1275), (0x00fd, 1233), (0x00fe, 1291), (0x00ff, 1233), (0x0374, 598),
    (0x0375, 600), (0x0376, 1561), (0x037a, 602), (0x037b, 1205), (0x037c, 1205), (0x037d, 1205),
    (0x037e, 702), (0x037f, 1197), (0x0384, 698), (0x0385, 1477), (0x0386, 1529), (0x0387, 684),
    (0x0388, 1628), (0x0389, 1914), (0x038a, 959), (0x038c, 1885), (0x038e, 1967), (0x038f, 1894),
    (0x0390, 633), (0x0391, 1529), (0x0392, 1355), (0x0393, 1193), (0x0394, 1521), (0x0395, 1244),
    (0x0396, 1360), (0x0397, 1530), (0x0398, 1578), (0x0399, 575), (0x039a, 1473), (0x039b, 1529),
    (0x039c, 1908), (0x039d, 1561), (0x039e, 1408), (0x039f, 1578), (0x03a0, 1530), (0x03a1, 1327),
    (0x03a3, 1420), (0x03a4, 1367), (0x03a5, 1497), (0x03a6, 1829), (0x03a7, 1512), (0x03a8, 1831),
    (0x03a9, 1601), (0x03aa, 575), (0x03ab, 1497), (0x03ac, 1421), (0x03ad, 1142), (0x03ae, 1275),
    (0x03af, 633), (0x03b0, 1274), (0x03b1, 1421), (0x03b2, 1248), (0x03b3, 1228), (0x03b4, 1239),
    (0x03b5, 1142), (0x03b6, 1094), (0x03b7, 1275), (0x03b8, 1284), (0x03b9, 633), (0x03ba, 1201),
    (0x03bb, 1398), (0x03bc, 1286), (0x03bd, 1259), (0x03be, 1143), (0x03bf, 1256), (0x03c0, 1309),
    (0x03c1, 1291), (0x03c2, 1203), (0x03c3, 1333), (0x03c4, 1045), (0x03c5, 1274), (0x03c6, 1635),
    (0x03c7, 1233), (0x03c8, 1700), (0x03c9, 1707), (0x03ca, 633), (0x03cb, 1274), (0x03cc, 1256),
    (0x03cd, 1274), (0x03ce, 1707), (0x03cf, 1473), (0x03d0, 1371), (0x03d1, 1368), (0x03d2, 1452),
    (0x03d3, 1888), (0x03d4, 1452), (0x03d5, 1535), (0x03d6, 1899), (0x03d7, 1201), (0x03dc, 1223),
    (0x03dd, 1100), (0x03f0, 1560), (0x03f1, 1371), (0x03f2, 1205), (0x03f3, 555), (0x03f4, 1578),
    (0x03f5, 980), (0x03f6, 980), (0x03f9, 1515), (0x03fa, 1908), (0x03fc, 1291), (0x03fd, 1515),
    (0x03fe, 1515), (0x03ff, 1515), (0x2013, 1024), (0x2014, 2048), (0x2018, 636), (0x2019, 636),
    (0x201c, 1106), (0x201d, 1089), (0x2022, 971), (0x2026, 2052), (0x2030, 2846), (0x2032, 562),
    (0x2033, 1121), (0x2070, 1015), (0x2074, 997), (0x2075, 971), (0x2076, 973), (0x2077, 869),
    (0x2078, 985), (0x2079, 973), (0x207a, 963), (0x207b, 964), (0x2080, 1015), (0x2081, 682),
    (0x2082, 942), (0x2083, 966), (0x2084, 997), (0x2085, 971), (0x2086, 973), (0x2087, 869),
    (0x2088, 985), (0x2089, 973), (0x2190, 1954), (0x2191, 1854), (0x2192, 1954), (0x2193, 1854),
    (0x2194, 2746), (0x2195, 1757), (0x2196, 1603), (0x2197, 1603), (0x2198, 1603), (0x2199, 1603),
    (0x219a, 1626), (0x219b, 1626), (0x219c, 1749), (0x219d, 1749), (0x219e, 1831), (0x219f, 893),
    (0x21a0, 1831), (0x21a1, 893), (0x21a2, 1642), (0x21a3, 1642), (0x21a4, 1626), (0x21a5, 893),
    (0x21a6, 1626), (0x21a7, 893), (0x21a8, 893), (0x21a9, 2373), (0x21aa, 2373), (0x21ab, 1761),
    (0x21ac, 1761), (0x21ad, 1952), (0x21ae, 1810), (0x21b0, 2072), (0x21b1, 2072), (0x21b2, 1237),
    (0x21b3, 2072), (0x21b4, 1835), (0x21b5, 2072), (0x21b6, 2091), (0x21b7, 2091), (0x21b8, 1692),
    (0x21b9, 2032), (0x21ba, 1954), (0x21bb, 1954), (0x21bc, 1626), (0x21bd, 1626), (0x21be, 893),
    (0x21bf, 893), (0x21c0, 1626), (0x21c1, 1626), (0x21c2, 893), (0x21c3, 893), (0x21c4, 1626),
    (0x21c5, 1565), (0x21c6, 1626), (0x21c7, 1626), (0x21c8, 1565), (0x21c9, 1626), (0x21ca, 1565),
    (0x21cb, 1626), (0x21cc, 1626), (0x21cd, 1647), (0x21ce, 2015), (0x21cf, 1647), (0x21d0, 1954),
    (0x21d1, 1257), (0x21d2, 1954), (0x21d3, 1257), (0x21d4, 2746), (0x21d5, 1257), (0x21d6, 1384),
    (0x21d7, 1384), (0x21d8, 1384), (0x21d9, 1384), (0x21da, 1647), (0x21db, 1647), (0x21dc, 1774),
    (0x21dd, 1774), (0x21de, 1301), (0x21df, 1301), (0x21e0, 1710), (0x21e1, 893), (0x21e2, 1710),
    (0x21e3, 893), (0x21e4, 2149), (0x21e5, 2149), (0x21e7, 2253), (0x21ea, 2253), (0x21f1, 1837),
    (0x21f2, 1837), (0x21f4, 1731), (0x21f5, 1565), (0x21f6, 1626), (0x21f7, 1626), (0x21f8, 1626),
    (0x21f9, 1810), (0x21fa, 1626), (0x21fb, 1626), (0x21fc, 1810), (0x21fd, 1753), (0x21fe, 1753),
    (0x21ff, 2060), (0x2200, 1536), (0x2201, 883), (0x2202, 1371), (0x2203, 1176), (0x2204, 1176),
    (0x2205, 1412), (0x2206, 1521), (0x2207, 1190), (0x2208, 1432), (0x2209, 1432), (0x220a, 1139),
    (0x220b, 1432), (0x220c, 1432), (0x220d, 1139), (0x220e, 1143), (0x220f, 1570), (0x2210, 1329),
    (0x2211, 1514), (0x2212, 1390), (0x2213, 1171), (0x2214, 1171), (0x2215, 825), (0x2216, 825),
    (0x2217, 934), (0x2218, 782), (0x2219, 594), (0x221a, 1371), (0x221b, 1239), (0x221c, 1239),
    (0x221d, 1106), (0x221e, 2008), (0x221f, 1335), (0x2220, 1335), (0x2221, 1335), (0x2222, 1171),
    (0x2223, 354), (0x2224, 883), (0x2225, 737), (0x2226, 1262), (0x2227, 1462), (0x2228, 1462),
    (0x2229, 1401), (0x222a, 1401), (0x222b, 801), (0x222c, 1323), (0x222d, 1853), (0x222e, 1116),
    (0x222f, 1683), (0x2230, 2275), (0x2231, 1200), (0x2232, 1198), (0x2233, 1227), (0x2234, 1169),
    (0x2235, 1171), (0x2236, 684), (0x2237, 1171), (0x2238, 1171), (0x2239, 1264), (0x223a, 1171),
    (0x223b, 1171), (0x223c, 1171), (0x223d, 1171), (0x223e, 1425), (0x223f, 1366), (0x2240, 498),
    (0x2241, 1171), (0x2242, 1171), (0x2243, 1171), (0x2244, 1171), (0x2245, 1171), (0x2246, 1171),
    (0x2247, 1171), (0x2248, 1390), (0x2249, 1171), (0x224a, 1171), (0x224b, 1171), (0x224c, 1171),
    (0x224d, 1171), (0x224e, 1171), (0x224f, 1171), (0x2250, 1171), (0x2251, 1171), (0x2252, 1171),
    (0x2253, 1171), (0x2254, 1264), (0x2255, 1264), (0x2256, 1171), (0x2257, 1171), (0x2258, 1171),
    (0x2259, 1171), (0x225a, 1171), (0x225b, 1171), (0x225c, 1171), (0x225d, 1171), (0x225e, 1171),
    (0x225f, 1171), (0x2260, 1390), (0x2261, 1171), (0x2262, 1171), (0x2263, 1171), (0x2264, 1390),
    (0x2265, 1390), (0x2266, 1171), (0x2267, 1171), (0x2268, 1171), (0x2269, 1171), (0x226a, 1706),
    (0x226b, 1706), (0x226c, 713), (0x226d, 1171), (0x226e, 1171), (0x226f, 1171), (0x2270, 1171),
    (0x2271, 1171), (0x2272, 1171), (0x2273, 1171), (0x2274, 1171), (0x2275, 1171), (0x2276, 1171),
    (0x2277, 1171), (0x2278, 1171), (0x2279, 1171), (0x227a, 1110), (0x227b, 1110), (0x227c, 1110),
    (0x227d, 1110), (0x227e, 1171), (0x227f, 1171), (0x2280, 1110), (0x2281, 1110), (0x2282, 1432),
    (0x2283, 1432), (0x2284, 1432), (0x2285, 1432), (0x2286, 1432), (0x2287, 1432), (0x2288, 1432),
    (0x2289, 1432), (0x228a, 1432), (0x228b, 1432), (0x228c, 1319), (0x228d, 1319), (0x228e, 1319),
    (0x228f, 1171), (0x2290, 1319), (0x2291, 1319), (0x2292, 1319), (0x2293, 1319), (0x2294, 1319),
    (0x2295, 2846), (0x2296, 2846), (0x2297, 2846), (0x2298, 2846), (0x2299, 1518), (0x229a, 1901),
    (0x229b, 1901), (0x229c, 1901), (0x229d, 1901), (0x229e, 1548), (0x229f, 1548), (0x22a0, 1548),
    (0x22a1, 1548), (0x22a2, 1548), (0x22a3, 1548), (0x22a4, 1548), (0x22a5, 1548), (0x22a6, 952),
    (0x22a7, 961), (0x22a8, 1548), (0x22a9, 1712), (0x22aa, 1731), (0x22ab, 1712), (0x22ac, 1548),
    (0x22ad, 1548), (0x22ae, 1712), (0x22af, 1712), (0x22b0, 1161), (0x22b1, 1161), (0x22b2, 1171),
    (0x22b3, 1171), (0x22b4, 1171), (0x22b5, 1171), (0x22b6, 1872), (0x22b7, 1872), (0x22b8, 1630),
    (0x22b9, 1171), (0x22ba, 1139), (0x22bb, 1171), (0x22bc, 1171), (0x22bd, 1171), (0x22be, 1335),
    (0x22bf, 1440), (0x22c0, 1884), (0x22c1, 1884), (0x22c2, 1577), (0x22c3, 1577), (0x22c4, 1206),
    (0x22c5, 455), (0x22c6, 971), (0x22c7, 1171), (0x22c8, 1503), (0x22c9, 1503), (0x22ca, 1503),
    (0x22cb, 1507), (0x22cc, 1507), (0x22cd, 1171), (0x22ce, 1495), (0x22cf, 1495), (0x22d0, 1432),
    (0x22d1, 1432), (0x22d2, 1401), (0x22d3, 1401), (0x22d4, 1397), (0x22d5, 1229), (0x22d6, 1090),
    (0x22d7, 1090), (0x22d8, 2357), (0x22d9, 2357), (0x22da, 1171), (0x22db, 1171), (0x22dc, 1171),
    (0x22dd, 1171), (0x22de, 1110), (0x22df, 1110), (0x22e0, 1110), (0x22e1, 1110), (0x22e2, 1171),
    (0x22e3, 1171), (0x22e4, 1171), (0x22e5, 1171), (0x22e6, 1171), (0x22e7, 1171), (0x22e8, 1171),
    (0x22e9, 1171), (0x22ea, 1171), (0x22eb, 1171), (0x22ec, 1171), (0x22ed, 1171), (0x22ee, 455),
    (0x22ef, 1700), (0x22f0, 1712), (0x22f1, 1712), (0x22f2, 1671), (0x22f3, 1432), (0x22f4, 1139),
    (0x22f5, 1432), (0x22f6, 1432), (0x22f7, 1139), (0x22f8, 1432), (0x22f9, 1432), (0x22fa, 1671),
    (0x22fb, 1432), (0x22fc, 1139), (0x22fd, 1432), (0x22fe, 1139), (0x22ff, 1411),
];

/// The advance of a character in font units, in the regular or the bold font.
fn advance(c: char, weight: FontWeight) -> u16 {
    let (ascii, other) = match weight {
        FontWeight::Normal => (&ASCII, &OTHER[..]),
        FontWeight::Bold => (&ASCII_BOLD, &OTHER_BOLD[..]),
    };
    let code = c as u32;
    if (0x20..0x7f).contains(&code) {
        return ascii[(code - 0x20) as usize];
    }
    match other.binary_search_by_key(&code, |&(c, _)| c) {
        Ok(i) => other[i].1,
        Err(_) => ascii[(u32::from('0') - 0x20) as usize],
    }
}

/// The lines of a text: its parts between the `\n`.
pub(crate) fn lines(text: &str) -> std::str::Split<'_, char> {
    text.split('\n')
}

/// The number of lines of a text, at least one.
pub(crate) fn line_count(text: &str) -> usize {
    text.matches('\n').count() + 1
}

/// The pixels the lines of a text after its first take at a font size, 0 for one line.
pub(crate) fn extra_lines(text: &str, size: f64) -> f64 {
    (line_count(text) - 1) as f64 * LINE_HEIGHT * size
}

/// The width of a text in pixels at a font size in pixels, in Inter Regular: the width of its
/// widest line.
pub fn text_width(text: &str, size: f64) -> f64 {
    text_width_weighted(text, size, FontWeight::Normal)
}

/// The width of a text in pixels at a font size in pixels, in Inter of a weight (since 0.28.0):
/// the width of its widest line.
pub fn text_width_weighted(text: &str, size: f64, weight: FontWeight) -> f64 {
    let units = lines(text)
        .map(|line| {
            line.chars()
                .map(|c| u64::from(advance(c, weight)))
                .sum::<u64>()
        })
        .max()
        .unwrap_or(0);
    units as f64 * size / UNITS_PER_EM
}

/// The advance of every character of a monospace text in font sizes (since 0.31.0): Noto Sans
/// Mono, which the PNG draws it with, SF Mono and Menlo have 0.6 (Consolas 0.55).
pub const MONO_ADVANCE: f64 = 0.6;

/// The width of a text in pixels at a font size in pixels, in a family and a weight (since
/// 0.31.0): of Inter the width of [`text_width_weighted`], of a monospace font the characters
/// of its widest line times [`MONO_ADVANCE`] of the size, whatever the weight.
pub fn text_width_family(text: &str, size: f64, weight: FontWeight, family: FontFamily) -> f64 {
    match family {
        FontFamily::Sans => text_width_weighted(text, size, weight),
        FontFamily::Monospace => {
            let chars = lines(text).map(|l| l.chars().count()).max().unwrap_or(0);
            chars as f64 * MONO_ADVANCE * size
        }
    }
}

/// The width of a text SVG draws on one line (since 0.28.0): its newlines read as spaces, as an
/// SVG text without `tspan`s renders them. Tick labels and the texts of heatmap cells are drawn
/// so.
pub(crate) fn text_width_one_line(text: &str, size: f64, weight: FontWeight) -> f64 {
    text_width_weighted(&text.replace('\n', " "), size, weight)
}

/// The font size of a superscript of a tick label as a share of the size of its label
/// (matplotlib's mathtext shrinks a superscript to 0.7).
pub(crate) const SUP_SCALE: f64 = 0.7;
/// How far a superscript of a tick label is raised, a share of the size of its label.
pub(crate) const SUP_RISE: f64 = 0.4;

/// The runs of a line of a tick label (since 0.31.0): its plain text and its superscripts,
/// each written `^{...}` (`10^{−3}`), a superscript `true`. A `^` without a closing brace is
/// plain text.
pub(crate) fn runs(line: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("^{") {
        let Some(len) = rest[start + 2..].find('}') else {
            break;
        };
        if start > 0 {
            out.push((&rest[..start], false));
        }
        out.push((&rest[start + 2..start + 2 + len], true));
        rest = &rest[start + 3 + len..];
    }
    if !rest.is_empty() || out.is_empty() {
        out.push((rest, false));
    }
    out
}

/// The text of a line of a tick label without its markup: its runs joined.
pub(crate) fn plain(line: &str) -> String {
    runs(line).into_iter().map(|(t, _)| t).collect()
}

/// The width of a tick label at a font size (since 0.31.0): of its widest line, a superscript
/// measured at [`SUP_SCALE`] of the size.
pub(crate) fn label_width(text: &str, size: f64, weight: FontWeight) -> f64 {
    lines(text)
        .map(|line| {
            runs(line)
                .into_iter()
                .map(|(t, sup)| {
                    let s = if sup { size * SUP_SCALE } else { size };
                    text_width_weighted(t, s, weight)
                })
                .sum::<f64>()
        })
        .fold(0.0, f64::max)
}

/// The height of the line box of a tick label at a font size: one font size per line and
/// [`LINE_HEIGHT`] between lines.
pub(crate) fn label_height(text: &str, size: f64) -> f64 {
    size + extra_lines(text, size)
}

/// The widest line of a tick label without its markup, which Charton is given to measure.
pub(crate) fn widest_line(text: &str, size: f64, weight: FontWeight) -> String {
    lines(text)
        .max_by(|a, b| label_width(a, size, weight).total_cmp(&label_width(b, size, weight)))
        .map(plain)
        .unwrap_or_default()
}

/// The extent across its axis of a tick label of a width, rotated by `angle` degrees: the side
/// of its rotated line box (one em, the font size, per line, see [`label_height`]) which the
/// axis band reserves, as Charton computes it, with the sine and cosine of `libm`.
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
    fn extents_of_rotated_labels() {
        assert_eq!(label_extent(20.0, 11.0, 0.0, true), 11.0);
        assert_eq!(label_extent(20.0, 11.0, 0.0, false), 20.0);
        assert!((label_extent(20.0, 11.0, 90.0, true) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn table_is_sorted() {
        assert!(OTHER.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(OTHER_BOLD.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(text_width("µ°²αβ", 10.0) > 0.0);
    }

    #[test]
    fn superscripts() {
        assert_eq!(runs("10^{−3}"), [("10", false), ("−3", true)]);
        assert_eq!(
            runs("2×10^{4} s"),
            [("2×10", false), ("4", true), (" s", false)]
        );
        assert_eq!(runs("a^b"), [("a^b", false)]);
        assert_eq!(runs(""), [("", false)]);
        assert_eq!(plain("10^{−3}"), "10−3");
        let w = label_width("10^{2}", 10.0, FontWeight::Normal);
        let expected = text_width("10", 10.0) + text_width("2", 7.0);
        assert!((w - expected).abs() < 1e-12);
        assert_eq!(label_height("0\nfasted", 10.0), 22.0);
        assert_eq!(label_height("1", 10.0), 10.0);
    }

    #[test]
    fn bold_and_lines() {
        // fontTools on Inter-Bold.ttf at 13 px
        let bold = text_width_weighted("Smith2001", 13.0, FontWeight::Bold);
        assert!((bold - 68.08).abs() < 0.01, "{bold}");
        assert!(bold > text_width("Smith2001", 13.0));
        assert_eq!(text_width("a\nbbb\ncc", 10.0), text_width("bbb", 10.0));
        assert_eq!(line_count("a\nb\n"), 3);
        assert_eq!(extra_lines("one", 10.0), 0.0);
        assert!((extra_lines("one\ntwo", 10.0) - 12.0).abs() < 1e-12);
    }
}
