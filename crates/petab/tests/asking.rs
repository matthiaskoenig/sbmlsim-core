//! The fit of a problem driven ask/tell, as a page drives it, is the fit of the batch
//! ([`Executor::fit`]): the definition of [`Plan::definition`], its simulations run by a host
//! (here the native engine, which the batch runs as well) and the same optimum, cost and
//! counts to the bit.
//!
//! The problems are the conversion problem of the live demo of the documentation
//! (`docs/demo/conversion`) and every case of the PEtab test suite whose fit the engine
//! carries, when the suite is downloaded (see `tests/testsuite.rs`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sbmlsim_abi::{EngineError, SimulationResult};
use sbmlsim_fit::{Algorithm, Ask, Asking, Evolution, Fit, Opening, Settings, Simulation};
use sbmlsim_host::{LoadedModel, Runtime};
use sbmlsim_petab::{Executor, Options, PetabProblem, Plan};

/// The commit of the suite, the one of `tests/testsuite.rs`.
const COMMIT: &str = "d09985e7bb03280b461599a24337137533d931e2";

fn suite() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SBMLSIM_PETAB_SUITE_PATH") {
        return Some(PathBuf::from(path));
    }
    let cache = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .ok()?;
    let path = cache.join("sbmlsim").join("petab-test-suite").join(COMMIT);
    path.join("v2.0.0").is_dir().then_some(path)
}

/// The models of a plan, compiled and loaded as the executor loads them.
fn models(plan: &Plan, runtime: &Runtime) -> Vec<Arc<LoadedModel>> {
    (0..plan.models.len())
        .map(|k| {
            let compiled = plan.compile(k).unwrap();
            Arc::new(runtime.load(&compiled.wasm, &compiled.description).unwrap())
        })
        .collect()
}

/// The fit of a plan driven ask/tell, its simulations run on the native engine.
fn asked(plan: &Plan, models: &[Arc<LoadedModel>], settings: &Settings) -> Fit {
    let definition = plan.definition(&plan.nominal()).unwrap();
    let ids: Vec<String> = plan.models.iter().map(|model| model.id.clone()).collect();
    let mut asking = Asking::open(Opening::new(definition, &ids).unwrap(), settings.clone());
    let mut instances: Vec<_> = models
        .iter()
        .map(|model| model.instantiate_native().unwrap())
        .collect();
    let mut simulate = |simulations: &[Simulation]| -> Vec<Result<SimulationResult, EngineError>> {
        simulations
            .iter()
            .map(|simulation| instances[simulation.model].simulate(&simulation.request))
            .collect()
    };
    loop {
        match asking.ask().unwrap() {
            Ask::Simulate(simulations) => asking.tell(simulate(&simulations)).unwrap(),
            Ask::Done(fit) => return fit,
        }
    }
}

/// Fits a plan on the batch and ask/tell and compares the two; `None` for a plan whose fit
/// the engine does not carry.
fn compare(name: &str, plan: Plan, runtime: &Runtime, settings: &Settings) -> Option<Fit> {
    if plan.definition(&plan.nominal()).is_err() {
        return None;
    }
    let loaded = models(&plan, runtime);
    let executor = Executor::with_runtime(plan.clone(), runtime).unwrap();
    let batch = match executor.fit(settings, 2) {
        Ok(answer) => answer.fit,
        Err(err) => {
            eprintln!("  {name}: no fit on the batch: {err}");
            return None;
        }
    };
    let fit = asked(&plan, &loaded, settings);
    assert_eq!(batch.x, fit.x, "{name}");
    assert_eq!(batch.cost.to_bits(), fit.cost.to_bits(), "{name}");
    assert_eq!(batch.iterations, fit.iterations, "{name}");
    assert_eq!(batch.evaluations, fit.evaluations, "{name}");
    assert_eq!(batch.jacobians, fit.jacobians, "{name}");
    assert_eq!(batch.status, fit.status, "{name}");
    Some(fit)
}

fn demo() -> Plan {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/demo/conversion");
    let problem = PetabProblem::from_path(directory.join("problem.yaml")).unwrap();
    Plan::new(&problem, &Options::default()).unwrap()
}

#[test]
fn the_fit_of_the_demo_asked_for_its_simulations_is_the_one_of_the_batch() {
    let runtime = Runtime::embedded().unwrap();
    let fit = compare("demo", demo(), &runtime, &Settings::default()).unwrap();
    assert!(fit.iterations > 2, "{fit:?}");
    let evolution = Settings {
        algorithm: Algorithm::DifferentialEvolution(Evolution {
            seed: 3,
            population: Some(10),
            ..Evolution::default()
        }),
        max_iterations: 8,
        ..Settings::default()
    };
    compare("demo, evolution", demo(), &runtime, &evolution).unwrap();
    let levenberg = Settings {
        algorithm: Algorithm::LevenbergMarquardt,
        ..Settings::default()
    };
    compare("demo, Levenberg-Marquardt", demo(), &runtime, &levenberg).unwrap();
}

#[test]
fn the_fit_of_every_case_of_the_test_suite_asked_for_its_simulations_is_the_one_of_the_batch() {
    let Some(suite) = suite() else {
        eprintln!("the PEtab test suite {COMMIT} is not downloaded");
        return;
    };
    let runtime = Runtime::embedded().unwrap();
    let mut fitted = 0;
    for group in ["v2.0.0/sbml", "v1.0.0-converted/sbml"] {
        let Ok(entries) = std::fs::read_dir(suite.join(group)) else {
            continue;
        };
        let mut cases: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        cases.sort();
        for case in cases {
            let Some(id) = case
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let Ok(problem) = PetabProblem::from_path(case.join(format!("_{id}.yaml"))) else {
                continue;
            };
            let Ok(plan) = Plan::new(&problem, &Options::default()) else {
                continue;
            };
            if compare(
                &format!("{group}/{id}"),
                plan,
                &runtime,
                &Settings::default(),
            )
            .is_some()
            {
                fitted += 1;
            }
        }
    }
    eprintln!("{fitted} cases fitted on the batch and ask/tell alike");
    assert!(fitted > 0);
}
