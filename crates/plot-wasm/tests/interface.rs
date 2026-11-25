//! The C ABI of `plot.wasm`, driven on the host as a JavaScript host drives it.
//!
//! A test writes a spec into a buffer of `allocate`, calls `render`, `geometry` or `pick` with a
//! buffer of two words, reads the result out of the memory and frees every buffer, which is what
//! a page does with the memory of the module. The same functions compiled for
//! `wasm32-unknown-unknown` give the same bytes (`check_golden.mjs` of this crate runs the
//! module in Node against the golden files of `sbmlsim-plot`).

use std::path::PathBuf;
use std::slice;

use sbmlsim_plot::{Geometry, Pick, parse};
use sbmlsim_plot_wasm::{allocate, deallocate, geometry, pick, render};

/// The fixtures of the renderer, `crates/plot/tests/specs/*.json`, by name, sorted.
fn fixtures() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plot/tests/specs");
    let mut specs: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.expect("an entry of the fixtures").path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            (name, text)
        })
        .collect();
    specs.sort();
    assert!(!specs.is_empty(), "no fixtures in {}", dir.display());
    specs
}

/// The bytes of a buffer of a result, which is empty when the interface gives none.
///
/// # Safety
///
/// `pointer` and `len` are a buffer of a result which was not freed yet.
unsafe fn bytes_of(pointer: usize, len: usize) -> Vec<u8> {
    if pointer == 0 || len == 0 {
        return Vec::new();
    }
    // SAFETY: the caller gives a buffer of the interface of exactly this length
    unsafe { slice::from_raw_parts(pointer as *const u8, len).to_vec() }
}

/// Calls a function of the interface on a spec and gives its status and its result as text,
/// freeing every buffer.
fn call(spec: &str, function: impl FnOnce(*const u8, usize, *mut usize) -> u32) -> (u32, String) {
    call_bytes(spec.as_bytes(), function)
}

/// [`call`] on the bytes of a spec, which need not be UTF-8.
fn call_bytes(
    bytes: &[u8],
    function: impl FnOnce(*const u8, usize, *mut usize) -> u32,
) -> (u32, String) {
    // SAFETY: the buffers are the ones of the interface and are freed below
    unsafe {
        let input = allocate(bytes.len());
        if !bytes.is_empty() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), input, bytes.len());
        }
        let words = allocate(2 * size_of::<usize>()).cast::<usize>();
        let status = function(input, bytes.len(), words);
        let result = bytes_of(*words, *words.add(1));
        deallocate(*words as *mut u8, *words.add(1));
        deallocate(words.cast::<u8>(), 2 * size_of::<usize>());
        deallocate(input, bytes.len());
        (
            status,
            String::from_utf8(result).expect("a result is UTF-8"),
        )
    }
}

/// [`render`] through the interface.
fn render_through(spec: &str) -> (u32, String) {
    // SAFETY: `call` gives the buffers the interface documents
    call(spec, |p, n, out| unsafe { render(p, n, out) })
}

#[test]
fn render_matches_native() {
    for (name, spec) in fixtures() {
        let figure = parse(&spec).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let (status, svg) = render_through(&spec);
        assert_eq!(status, 1, "{name}: {svg}");
        assert_eq!(
            svg,
            sbmlsim_plot::render_svg(&figure),
            "{name}: the bytes of the interface are the native ones"
        );
    }
}

#[test]
fn invalid_spec_returns_zero_and_message() {
    let (_, curve) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "curve")
        .expect("the fixture curve");
    let invalid = curve.replace("figure/1", "figure/2");
    let (status, message) = render_through(&invalid);
    assert_eq!(status, 0, "{message}");
    assert!(!message.is_empty(), "a refused spec has a message");
    assert!(
        message.starts_with("contract: "),
        "the path and the message: {message}"
    );
    // the same message from each function of the interface
    // SAFETY: `call` gives the buffers the interface documents
    let (status, other) = call(&invalid, |p, n, out| unsafe { geometry(p, n, out) });
    assert_eq!((status, other.as_str()), (0, message.as_str()));
    // SAFETY: as above
    let (status, other) = call(&invalid, |p, n, out| unsafe {
        pick(p, n, 0.0, 0.0, 8.0, out)
    });
    assert_eq!((status, other.as_str()), (0, message.as_str()));
    // no spec and no UTF-8 are refused as well
    let (status, message) = render_through("");
    assert_eq!(status, 0, "{message}");
    assert!(!message.is_empty());
    // SAFETY: `call` gives the buffers the interface documents
    let (status, message) = call_bytes(&[0xff, 0xfe], |p, n, out| unsafe { render(p, n, out) });
    assert_eq!((status, message.as_str()), (0, "the spec is not UTF-8"));
}

#[test]
fn geometry_json_roundtrip() {
    for (name, spec) in fixtures() {
        let figure = parse(&spec).unwrap();
        // SAFETY: `call` gives the buffers the interface documents
        let (status, json) = call(&spec, |p, n, out| unsafe { geometry(p, n, out) });
        assert_eq!(status, 1, "{name}: {json}");
        let read: Geometry = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(read, sbmlsim_plot::geometry(&figure), "{name}");
    }
}

#[test]
fn pick_json_roundtrip() {
    let (_, spec) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "curve")
        .expect("the fixture curve");
    let figure = parse(&spec).unwrap();
    let panel = sbmlsim_plot::geometry(&figure).plots[0].rect;
    let (x, y) = (panel.x + panel.width / 2.0, panel.y + panel.height / 2.0);
    let radius = panel.width + panel.height;
    // SAFETY: `call` gives the buffers the interface documents
    let (status, json) = call(&spec, |p, n, out| unsafe { pick(p, n, x, y, radius, out) });
    assert_eq!(status, 1, "{json}");
    let read: Option<Pick> = serde_json::from_str(&json).expect("a pick is JSON");
    let native = sbmlsim_plot::pick(&figure, x, y, radius);
    assert!(native.is_some(), "the middle of the panel picks a point");
    assert_eq!(read, native);
    // nothing under the pointer is the JSON null
    // SAFETY: as above
    let (status, json) = call(&spec, |p, n, out| unsafe {
        pick(p, n, -1000.0, -1000.0, 1.0, out)
    });
    assert_eq!((status, json.as_str()), (1, "null"));
}
