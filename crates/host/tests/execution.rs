//! End-to-end containment of model failures, with a subprocess watchdog so regressions
//! cannot abort or hang the test runner.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(feature = "native")]
use sbmlsim_abi::EngineError;
use sbmlsim_abi::{ModelDescription, SimulationRequest};
use sbmlsim_host::Runtime;

fn fixtures(body: &str) -> (Vec<u8>, ModelDescription, SimulationRequest) {
    let wat = include_str!("decay.wat").replace(
        "(f64.store (local.get $ydot)\n      (f64.mul (f64.neg (f64.load (local.get $p))) (f64.load (local.get $y))))",
        body,
    );
    (
        wat::parse_str(wat).unwrap(),
        serde_json::from_str(include_str!("decay-model/model.json")).unwrap(),
        serde_json::from_str(include_str!("decay-model/request.json")).unwrap(),
    )
}

fn child(name: &str, run: impl FnOnce()) {
    const ENV: &str = "SBMLSIM_EXECUTION_TEST_CHILD";
    if std::env::var(ENV).as_deref() == Ok(name) {
        run();
        return;
    }
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(ENV, name)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = process.try_wait().unwrap() {
            assert!(status.success(), "{name} exited with {status}");
            break;
        }
        // Each child first compiles the embedded runtime. Concurrent tests on a busy
        // runner can spend more than 15 seconds there; the 20 ms model deadlines are
        // checked independently after compilation.
        if started.elapsed() > Duration::from_secs(60) {
            process.kill().unwrap();
            process.wait().unwrap();
            panic!("{name} did not contain the model within the watchdog limit");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[cfg(feature = "native")]
fn native_trap_is_contained() {
    child("native_trap_is_contained", || {
        let (wasm, desc, req) = fixtures("unreachable");
        let mut instance = Runtime::embedded()
            .unwrap()
            .instantiate_native(&wasm, &desc)
            .unwrap();
        let error = instance.simulate(&req).unwrap_err();
        assert!(
            matches!(error, EngineError::Internal(ref message) if message.contains("trapped")),
            "{error}"
        );
        assert!(matches!(
            instance.simulate(&req),
            Err(EngineError::Internal(_))
        ));
    });
}

#[test]
#[cfg(feature = "native")]
fn deadline_interrupts_wasm_and_native_rhs() {
    child("deadline_interrupts_wasm_and_native_rhs", || {
        let (wasm, desc, req) = fixtures("(loop $forever (br $forever))");
        let runtime = Runtime::embedded()
            .unwrap()
            .with_execution_timeout(Some(Duration::from_millis(20)));
        let loaded = runtime.load(&wasm, &desc).unwrap();
        let mut native = runtime.instantiate_native(&wasm, &desc).unwrap();
        drop(runtime);
        let mut instance = loaded.instantiate().unwrap();
        drop(loaded);
        for result in [instance.simulate(&req), native.simulate(&req)] {
            assert!(
                matches!(result, Err(EngineError::Internal(ref message)) if message.contains("interrupt")),
                "{result:?}"
            );
        }
        assert!(matches!(
            instance.simulate(&req),
            Err(EngineError::Internal(_))
        ));
        assert!(matches!(
            native.simulate(&req),
            Err(EngineError::Internal(_))
        ));
    });
}

#[test]
fn deadline_covers_loading_and_direct_model_functions() {
    child("deadline_covers_loading_and_direct_model_functions", || {
        let (wasm, desc, _) = fixtures("(loop $forever (br $forever))");
        let runtime = Runtime::embedded()
            .unwrap()
            .with_execution_timeout(Some(Duration::from_millis(20)));
        let mut functions = runtime.model_functions(&wasm, &desc).unwrap();
        let error = functions.rhs(0.0, &[1.0], &[0.5, 0.0]).unwrap_err();
        assert!(format!("{error:#}").contains("interrupt"));
        assert!(
            functions
                .jac(0.0, &[1.0], &[0.5, 0.0])
                .unwrap_err()
                .to_string()
                .contains("trapped before")
        );
        let wat = include_str!("decay.wat").replace(
            "(i32.store (local.get $out) (i32.const 1))",
            "(loop $forever (br $forever))",
        );
        let wasm = wat::parse_str(wat).unwrap();
        let error = runtime.instantiate(&wasm, &desc).err().unwrap();
        assert!(format!("{error:#}").contains("interrupt"));
    });
}

#[test]
fn models_cannot_allocate_private_tables() {
    let wat = include_str!("decay.wat").replace(
        ";; t_half = ln(2) / k, from p[0] into p[1]",
        "(table 1000000 funcref)",
    );
    let wasm = wat::parse_str(wat).unwrap();
    let desc = serde_json::from_str(include_str!("decay-model/model.json")).unwrap();
    let error = Runtime::embedded()
        .unwrap()
        .load(&wasm, &desc)
        .err()
        .unwrap();
    assert!(error.to_string().contains("table"));
}

#[test]
fn idle_accessors_start_their_own_deadlines() {
    let (wasm, desc, req) = fixtures("(f64.store (local.get $ydot) (f64.const 0))");
    let runtime = Runtime::embedded()
        .unwrap()
        .with_execution_timeout(Some(Duration::from_millis(20)));
    let mut instance = runtime.instantiate(&wasm, &desc).unwrap();
    instance.simulate(&req).unwrap();
    std::thread::sleep(Duration::from_millis(60));
    assert!(instance.stats().is_ok());
    std::thread::sleep(Duration::from_millis(60));
    assert!(instance.output_address().is_ok());
    assert!(instance.simulate(&req).is_ok());
}

#[test]
#[cfg(feature = "native")]
fn native_model_memory_growth_obeys_the_runtime_limit() {
    let wat = include_str!("decay.wat").replace(
        "(f64.store (local.get $y) (f64.const 1))",
        "(f64.store (local.get $y) (f64.convert_i32_s (memory.grow (i32.const 64))))",
    );
    let wasm = wat::parse_str(wat).unwrap();
    let desc = serde_json::from_str(include_str!("decay-model/model.json")).unwrap();
    let mut req: SimulationRequest =
        serde_json::from_str(include_str!("decay-model/request.json")).unwrap();
    req.end = 0.0;
    req.output = sbmlsim_abi::Output::Times(vec![0.0]);
    let runtime = Runtime::embedded().unwrap().with_max_memory(32 * 65536);
    let mut instance = runtime.instantiate_native(&wasm, &desc).unwrap();
    // memory.grow reports -1 instead of allocating four MiB outside the policy.
    assert_eq!(instance.simulate(&req).unwrap().values, [0.0, -1.0]);
}
