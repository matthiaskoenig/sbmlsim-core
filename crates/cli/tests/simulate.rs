//! `sbmlsim-core simulate` on the decay model of `crates/host/tests/decay-model`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sbmlsim_abi::{ModelDescription, SimulationRequest};
use sbmlsim_host::Runtime;

/// The model directory of the decay model.
fn model_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../host/tests/decay-model")
}

/// Runs the command line with `args`.
fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sbmlsim-core"))
        .args(args)
        .output()
        .expect("the command runs")
}

/// The CSV of the result of the request of the model directory, through the host.
fn expected_rows() -> (Vec<String>, Vec<Vec<f64>>) {
    let dir = model_dir();
    let desc: ModelDescription =
        serde_json::from_slice(&std::fs::read(dir.join("model.json")).unwrap()).unwrap();
    let req: SimulationRequest =
        serde_json::from_slice(&std::fs::read(dir.join("request.json")).unwrap()).unwrap();
    let wasm = std::fs::read(dir.join("model.wasm")).unwrap();
    let result = Runtime::embedded()
        .unwrap()
        .instantiate(&wasm, &desc)
        .unwrap()
        .simulate(&req)
        .unwrap();
    let k = result.columns.len();
    let rows = result.values.chunks(k).map(<[f64]>::to_vec).collect();
    (result.columns, rows)
}

/// Asserts that a CSV is the expected result, every number read back bitwise.
fn assert_csv(csv: &str) {
    let (columns, rows) = expected_rows();
    let mut lines = csv.lines();
    assert_eq!(lines.next().unwrap(), columns.join(","));
    let read: Vec<Vec<f64>> = lines
        .map(|line| line.split(',').map(|v| v.parse().unwrap()).collect())
        .collect();
    assert_eq!(read.len(), 11);
    for (r, e) in read.iter().zip(&rows) {
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(r), bits(e));
    }
}

#[test]
fn simulate_writes_the_csv() {
    let dir = model_dir();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("decay.csv");
    let _ = std::fs::remove_file(&out);
    let output = run(&[
        "simulate".as_ref(),
        dir.as_os_str(),
        dir.join("request.json").as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    let csv = std::fs::read_to_string(&out).unwrap();
    assert!(csv.starts_with("time,y\n0,1\n1,"), "{csv}");
    assert_csv(&csv);
    // without -o the CSV goes to stdout
    let output = run(&[
        "simulate".as_ref(),
        dir.as_os_str(),
        dir.join("request.json").as_os_str(),
    ]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), csv);
}

#[test]
fn errors_are_reported() {
    let dir = model_dir();
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let request = std::fs::read_to_string(dir.join("request.json")).unwrap();
    let bad = tmp.join("no-steps.json");
    std::fs::write(&bad, request.replace("\"steps\": 10", "\"steps\": 0")).unwrap();
    let output = run(&["simulate".as_ref(), dir.as_os_str(), bad.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("invalid request"), "{stderr}");
    // a directory without a model
    let output = run(&[
        "simulate".as_ref(),
        tmp.as_os_str(),
        dir.join("request.json").as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("model.wasm"), "{stderr}");
    // a usage error
    let output = run(&["simulate".as_ref()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn compile_writes_a_model_which_simulates() {
    let sbml = Path::new(env!("CARGO_MANIFEST_DIR")).join("../host/tests/decay.xml");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("compiled-decay");
    let _ = std::fs::remove_dir_all(&out);
    let output = run(&[
        "compile".as_ref(),
        sbml.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert!(output.status.success(), "{output:?}");
    let desc: ModelDescription =
        serde_json::from_slice(&std::fs::read(out.join("model.json")).unwrap()).unwrap();
    assert_eq!(desc.id, "decay");
    // the SBML of decay.wat computes its bits: the same CSV as the hand-written model
    let output = run(&[
        "simulate".as_ref(),
        out.as_os_str(),
        model_dir().join("request.json").as_os_str(),
    ]);
    assert!(output.status.success(), "{output:?}");
    assert_csv(&String::from_utf8(output.stdout).unwrap());
}

#[test]
fn compile_writes_a_kernel() {
    let sbml = Path::new(env!("CARGO_MANIFEST_DIR")).join("../host/tests/decay.xml");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("compiled-decay-kernel");
    let _ = std::fs::remove_dir_all(&out);
    let output = run(&[
        "compile".as_ref(),
        sbml.as_os_str(),
        "--kernel".as_ref(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert!(output.status.success(), "{output:?}");
    let compiled =
        sbmlsim_compiler::compile_kernel(&std::fs::read_to_string(&sbml).unwrap()).unwrap();
    assert_eq!(
        std::fs::read(out.join("kernel.wasm")).unwrap(),
        compiled.wasm
    );
    let description: sbmlsim_abi::KernelDescription =
        serde_json::from_slice(&std::fs::read(out.join("kernel.json")).unwrap()).unwrap();
    assert_eq!(description, compiled.description);
    assert!(!out.join("model.wasm").exists());
    // a kernel is always optimized
    let output = run(&[
        "compile".as_ref(),
        sbml.as_os_str(),
        "--kernel".as_ref(),
        "--no-optimize".as_ref(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn compile_drives_species() {
    let sbml =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/models/spt_liver_cell.xml");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("compiled-spt-driven");
    let _ = std::fs::remove_dir_all(&out);
    let output = run(&[
        "compile".as_ref(),
        sbml.as_os_str(),
        "--driven".as_ref(),
        "S_ext,P_ext".as_ref(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert!(output.status.success(), "{output:?}");
    let description: ModelDescription =
        serde_json::from_slice(&std::fs::read(out.join("model.json")).unwrap()).unwrap();
    let options = sbmlsim_compiler::CompileOptions {
        driven: vec!["S_ext".into(), "P_ext".into()],
        ..sbmlsim_compiler::CompileOptions::default()
    };
    let compiled =
        sbmlsim_compiler::compile(&std::fs::read_to_string(&sbml).unwrap(), &options).unwrap();
    assert_eq!(description, compiled.description);
    assert_eq!(
        std::fs::read(out.join("model.wasm")).unwrap(),
        compiled.wasm
    );
    let driven: Vec<&str> = description.driven.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(driven, ["S_ext", "P_ext"]);
    // the same species separated by a comma, or each with its own flag, and the model after the
    // flag
    for args in [
        vec!["--driven", "S_ext,P_ext"],
        vec!["--driven", "S_ext", "--driven", "P_ext"],
    ] {
        let _ = std::fs::remove_dir_all(&out);
        let mut command: Vec<&std::ffi::OsStr> = vec![std::ffi::OsStr::new("compile")];
        command.extend(args.iter().map(std::ffi::OsStr::new));
        command.extend([
            sbml.as_os_str(),
            std::ffi::OsStr::new("-o"),
            out.as_os_str(),
        ]);
        let output = run(&command);
        assert!(output.status.success(), "{args:?}: {output:?}");
        let again: ModelDescription =
            serde_json::from_slice(&std::fs::read(out.join("model.json")).unwrap()).unwrap();
        assert_eq!(again, description, "{args:?}");
    }
    // an unknown species is the error of the compiler, not of the command line
    let output = run(&[
        "compile".as_ref(),
        sbml.as_os_str(),
        "--driven".as_ref(),
        "Q".as_ref(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains('Q'),
        "{output:?}"
    );
    // the kernel takes no driven species yet
    let output = run(&[
        "compile".as_ref(),
        sbml.as_os_str(),
        "--kernel".as_ref(),
        "--driven".as_ref(),
        "S_ext".as_ref(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn compile_can_omit_parameter_derivatives_without_changing_simulation() {
    let sbml = Path::new(env!("CARGO_MANIFEST_DIR")).join("../host/tests/decay.xml");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("compiled-decay-ordinary");
    let output = run(&[
        "compile".as_ref(),
        sbml.as_os_str(),
        "--no-parameter-jacobian".as_ref(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert!(output.status.success(), "{output:?}");
    let description: ModelDescription =
        serde_json::from_slice(&std::fs::read(out.join("model.json")).unwrap()).unwrap();
    assert!(description.jacobian_p.is_none());
    let output = run(&[
        "simulate".as_ref(),
        out.as_os_str(),
        model_dir().join("request.json").as_os_str(),
    ]);
    assert!(output.status.success(), "{output:?}");
    assert_csv(&String::from_utf8(output.stdout).unwrap());
}

#[test]
fn compile_separates_an_unsolvable_from_an_overdetermined_algebraic_system() {
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let model = |parameters: &str, rules: &str| {
        format!(
            r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model><listOfParameters>{parameters}</listOfParameters><listOfRules>{rules}</listOfRules></model></sbml>"#
        )
    };
    let rule = |math: &str| {
        format!(
            r#"<algebraicRule><math xmlns="http://www.w3.org/1998/Math/MathML">{math}</math></algebraicRule>"#
        )
    };
    let minus =
        |id: &str, value: &str| format!("<apply><minus/><ci>{id}</ci><cn>{value}</cn></apply>");
    // the rule reads nothing it could determine, which is not an overdetermined system
    let unsolvable = model(
        r#"<parameter id="x" value="1" constant="true"/>"#,
        &rule("<ci>x</ci>"),
    );
    // two rules compete for the one variable they both read
    let overdetermined = model(
        r#"<parameter id="x" value="1" constant="false"/>"#,
        &format!("{}{}", rule(&minus("x", "1")), rule(&minus("x", "2"))),
    );
    for (case, body, expected) in [
        (
            "unsolvable",
            unsolvable,
            "algebraicRule #0 determines no variable: every entity it reads is already \
             determined (x is constant)",
        ),
        (
            "overdetermined",
            overdetermined,
            "overdetermined algebraic system: no independent variable for algebraicRule #1, \
             whose candidates x are taken by the other algebraic rules",
        ),
    ] {
        let sbml = tmp.join(format!("{case}.xml"));
        std::fs::write(&sbml, body).unwrap();
        let out = tmp.join(case);
        let output = run(&[
            "compile".as_ref(),
            sbml.as_os_str(),
            "-o".as_ref(),
            out.as_os_str(),
        ]);
        assert_eq!(output.status.code(), Some(1), "{case}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(expected), "{case}: {stderr}");
        assert!(!out.join("model.wasm").exists(), "{case}");
    }
}

#[test]
fn runtime_writes_the_modules_and_the_notices_of_the_engine() {
    let dir = std::env::temp_dir().join(format!("sbmlsim-core-runtime-{}", std::process::id()));
    let output = run(&["runtime".as_ref(), "-o".as_ref(), dir.as_os_str()]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read(dir.join("engine.wasm")).unwrap(),
        sbmlsim_host::ENGINE_WASM
    );
    assert_eq!(
        std::fs::read(dir.join("mathlib.wasm")).unwrap(),
        sbmlsim_host::MATHLIB_WASM
    );
    assert_eq!(
        std::fs::read(dir.join("compiler.wasm")).unwrap(),
        sbmlsim_host::COMPILER_WASM
    );
    assert_eq!(
        std::fs::read(dir.join("plot.wasm")).unwrap(),
        sbmlsim_host::PLOT_WASM
    );
    assert!(sbmlsim_host::PLOT_WASM.starts_with(b"\0asm"));
    let notices = std::fs::read_to_string(dir.join("THIRD_PARTY.md")).unwrap();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../THIRD_PARTY.md");
    assert_eq!(notices, std::fs::read_to_string(repository).unwrap());
    assert!(notices.contains("LGPL-2.1-or-later") && notices.contains("SUNDIALS"));
    std::fs::remove_dir_all(dir).unwrap();
}
