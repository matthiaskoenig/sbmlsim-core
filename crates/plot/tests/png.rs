//! The PNG of a figure: the SVG of the renderer rasterized with the bundled font.
#![cfg(feature = "png")]

mod support;

use sbmlsim_plot::{FigurePlan, parse, render_png};
use std::process::Command;
use support::spec_text;

/// The size a PNG says it has, from its IHDR.
fn size(png: &[u8]) -> (u32, u32) {
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
    (w, h)
}

/// The figure of the fixture `name`.
fn figure(name: &str) -> sbmlsim_plot::Figure {
    parse(&spec_text(name)).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn png_signature_and_size() {
    let f = figure("curve");
    let (w, h) = (f.width, f.height);
    let png = render_png(&f, 144.0).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        size(&png),
        ((w * 2.0).round() as u32, (h * 2.0).round() as u32)
    );
    let plan = FigurePlan::new(&f).render_png(&f, 72.0).unwrap();
    assert_eq!(size(&plan), (w.round() as u32, h.round() as u32));
}

#[test]
fn png_rejects_bad_dpi() {
    let f = figure("curve");
    assert!(render_png(&f, 0.0).is_err());
    assert!(render_png(&f, f64::NAN).is_err());
}

/// The digest of the PNG of the every-kind figure, printed by the child of the next test.
#[test]
fn png_child() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let png = render_png(&figure("every_kind"), 96.0).unwrap();
    let mut h = DefaultHasher::new();
    png.hash(&mut h);
    println!("DIGEST {}", h.finish());
}

fn digest(env: &[(&str, &str)]) -> String {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["png_child", "--exact", "--nocapture", "--test-threads=1"])
        .envs(env.iter().copied())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    text.lines()
        .find_map(|l| l.find("DIGEST ").map(|i| l[i..].to_string()))
        .unwrap_or_else(|| panic!("no digest: {text}"))
}

#[test]
fn png_is_the_same_without_a_fontconfig_or_home() {
    let plain = digest(&[]);
    let blind = digest(&[("FONTCONFIG_FILE", "/dev/null"), ("HOME", "/nonexistent")]);
    assert_eq!(plain, blind);
}

#[test]
fn png_refuses_more_than_the_pixel_cap() {
    let f = figure("curve");
    // 600 x 400 at dpi 72 * 17 is 10200 x 6800 pixels, more than 64 Mpx
    let e = render_png(&f, 72.0 * 17.0).unwrap_err();
    assert!(e.message.contains("10200 by 6800 pixels"), "{e}");
    assert!(e.message.contains("64 Mpx"), "{e}");
}

/// Renders the benchmark figure; its time is checked with `SBMLSIM_CORE_BENCH=1` only, as the
/// benchmarks of python and JavaScript are, since a loaded machine is no failure.
#[test]
fn png_time() {
    let f = figure("benchmark");
    render_png(&f, 144.0).unwrap();
    let start = std::time::Instant::now();
    let png = render_png(&f, 144.0).unwrap();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("png {} bytes in {ms:.1} ms", png.len());
    if !cfg!(debug_assertions) && std::env::var("SBMLSIM_CORE_BENCH").as_deref() == Ok("1") {
        assert!(ms < 100.0, "{ms} ms");
    }
}

/// The PNG of an image element (since 0.34.0): its pixels drawn over its extent, the left one
/// red and the right one blue, each a square at the nearest pixel.
#[test]
fn png_draws_an_image() {
    const TWO: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGO4pq4hX74FAAhtAnCJDgMlAAAAAElFTkSuQmCC";
    let spec = format!(
        r#"{{"contract": "figure/1", "width": 300, "height": 200,
            "grid": {{"rows": 1, "columns": 1}},
            "plots": [{{"id": "p", "row": 0, "column": 0, "legend": false,
                        "x": {{}}, "y": {{}},
                        "elements": [{{"id": "i", "kind": "image", "png": "{TWO}",
                                       "extent": [0, 2, 0, 1]}}]}}]}}"#
    );
    let f = parse(&spec).unwrap_or_else(|e| panic!("{e}"));
    let png = render_png(&f, 72.0).unwrap();
    let pixmap = resvg::tiny_skia::Pixmap::decode_png(&png).unwrap();
    let g = sbmlsim_plot::geometry(&f);
    let r = &g.plots[0].rect;
    let at = |share: f64| {
        let (x, y) = (
            (r.x + share * r.width) as u32,
            (r.y + r.height / 2.0) as u32,
        );
        let p = pixmap.pixel(x, y).unwrap();
        (p.red(), p.green(), p.blue())
    };
    assert_eq!(at(0.25), (0xd6, 0x27, 0x28));
    assert_eq!(at(0.75), (0x1f, 0x77, 0xb4));
}
