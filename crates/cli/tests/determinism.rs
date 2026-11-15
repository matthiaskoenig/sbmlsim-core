//! The compiler is deterministic: every case of the SBML Test Suite compiled twice (on other
//! threads, with the default limits and with limits which split its functions) gives the same
//! bytes of `model.wasm` and `model.json`.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use sbmlsim_cli::testsuite::description_json;
use sbmlsim_compiler::{CompileOptions, Limits, compile, compile_with_limits};

fn cases() -> Option<Vec<PathBuf>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cases");
    if !dir.join("00001/case.json").exists() {
        eprintln!("note: {} is absent; skipped", dir.display());
        return None;
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("model.xml").is_file())
        .collect();
    dirs.sort();
    Some(dirs)
}

/// The bytes of `model.wasm` and `model.json`.
type Bytes = (Vec<u8>, Vec<u8>);

/// The bytes of `model.wasm` and `model.json` of a model, `None` for one which does not compile.
fn bytes(sbml: &str, optimize: bool, limits: Option<Limits>) -> Option<Bytes> {
    let options = CompileOptions {
        optimize,
        ..CompileOptions::default()
    };
    let compiled = match limits {
        Some(limits) => compile_with_limits(sbml, &options, limits),
        None => compile(sbml, &options),
    }
    .ok()?;
    Some((
        compiled.wasm,
        description_json(&compiled.description).unwrap(),
    ))
}

#[test]
fn compiling_twice_gives_the_same_bytes() {
    let Some(dirs) = cases() else { return };
    let small = Limits {
        bytes: 160,
        locals: 12,
    };
    let configurations = [
        (true, None),
        (false, None),
        (true, Some(small)),
        (false, Some(small)),
    ];
    let first: Vec<Vec<Option<Bytes>>> = dirs
        .par_iter()
        .map(|dir| {
            let sbml = std::fs::read_to_string(dir.join("model.xml")).unwrap();
            configurations
                .iter()
                .map(|(optimize, limits)| bytes(&sbml, *optimize, *limits))
                .collect()
        })
        .collect();
    // the second time in the reverse order, so on other threads with other hash seeds
    let differ: Vec<String> = dirs
        .par_iter()
        .zip(&first)
        .rev()
        .filter_map(|(dir, first)| {
            let sbml = std::fs::read_to_string(dir.join("model.xml")).unwrap();
            let again: Vec<_> = configurations
                .iter()
                .map(|(optimize, limits)| bytes(&sbml, *optimize, *limits))
                .collect();
            (&again != first).then(|| dir.display().to_string())
        })
        .collect();
    let compiled = first.iter().flatten().filter(|b| b.is_some()).count();
    eprintln!("{compiled} modules compiled twice");
    assert!(compiled > 5000, "{compiled}");
    assert!(differ.is_empty(), "{differ:?}");
}
