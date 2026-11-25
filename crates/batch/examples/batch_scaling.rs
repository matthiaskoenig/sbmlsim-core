//! How a batch scales over the threads of a pool, measured in Rust without the bindings.
//!
//! ```text
//! cargo run --release -p sbmlsim-batch --example batch_scaling -- \
//!     target/bench-models/sbmlsim-hctz.xml [--atol FILE] [--points 1000] [--threads 1,2,4,8] \
//!     [--mode batch|fed|pools] [--slice 0] [--rounds 7] [--end 240] [--steps 100] \
//!     [--preinit PODOSE_hctz]
//! ```
//!
//! `batch` runs the points as one batch of `k` threads, in slices of `--slice` points per
//! thread when it is not 0; `fed` runs them as the python bindings do, read in ranges by the
//! calling thread while the pool simulates (`Workers::map_fed`); `pools` runs `k` threads of the
//! process, each with a pool of one thread of its own and the whole batch, which is what a
//! process shares when no pool is shared. The efficiency of `k` threads is the rate of the
//! points over `k` times the rate of one thread, the best of `--rounds` rounds each. Pin the
//! process (`taskset -c 0,2,4,6,8,10,12,14` on a machine of eight P-cores with SMT), since a
//! thread on an E-core or an SMT sibling is slower and not what is measured.

use std::sync::{Arc, Barrier};
use std::time::Instant;

use anyhow::Context;
use sbmlsim_abi::{IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, Tolerance};
use sbmlsim_batch::Workers;
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::{LoadedModel, Runtime};

struct Args {
    model: String,
    atol: Option<String>,
    points: usize,
    threads: Vec<usize>,
    mode: String,
    rounds: usize,
    end: f64,
    steps: u32,
    preinit: Option<String>,
    slice: usize,
}

fn args() -> anyhow::Result<Args> {
    let mut args = Args {
        model: String::new(),
        atol: None,
        points: 1000,
        threads: vec![1, 2, 4, 8],
        mode: "batch".into(),
        rounds: 7,
        end: 240.0,
        steps: 100,
        preinit: Some("PODOSE_hctz".into()),
        slice: 0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--atol" => args.atol = Some(value()?),
            "--points" => args.points = value()?.parse()?,
            "--threads" => {
                args.threads = value()?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--mode" => args.mode = value()?,
            "--slice" => args.slice = value()?.parse()?,
            "--rounds" => args.rounds = value()?.parse()?,
            "--end" => args.end = value()?.parse()?,
            "--steps" => args.steps = value()?.parse()?,
            "--preinit" => {
                let name = value()?;
                args.preinit = (!name.is_empty()).then_some(name);
            }
            _ if args.model.is_empty() => args.model = arg,
            _ => anyhow::bail!("unknown argument {arg}"),
        }
    }
    anyhow::ensure!(
        !args.model.is_empty(),
        "the SBML file of the model is missing"
    );
    Ok(args)
}

/// The requests of the batch: one oral dose per point, 101 output times, three selections.
fn requests(model: &LoadedModel, args: &Args) -> anyhow::Result<Vec<SimulationRequest>> {
    let desc = model.description();
    let absolute_tolerance = match &args.atol {
        Some(path) => {
            let atol: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(&std::fs::read_to_string(path)?)?;
            Tolerance::PerState(
                desc.states
                    .iter()
                    .map(|state| {
                        atol.get(&state.id)
                            .and_then(|v| v.as_f64())
                            .unwrap_or(1e-10)
                    })
                    .collect(),
            )
        }
        None => Tolerance::Scalar(1e-10),
    };
    let settings = IntegratorSettings {
        relative_tolerance: 1e-6,
        absolute_tolerance,
        maximum_num_steps: 20000,
        ..IntegratorSettings::default()
    };
    let selections: Vec<String> = std::iter::once("time".to_string())
        .chain(
            desc.selections
                .iter()
                .filter(|selection| selection.id.starts_with('['))
                .take(2)
                .map(|selection| selection.id.clone()),
        )
        .collect();
    Ok((0..args.points)
        .map(|at| SimulationRequest {
            contract: SIMULATION_CONTRACT.to_string(),
            start: 0.0,
            end: args.end,
            output: Output::Uniform { steps: args.steps },
            selections: selections.clone(),
            settings: settings.clone(),
            preinit: args
                .preinit
                .iter()
                .map(|name| (name.clone(), 12.5 * (1 + at % 8) as f64))
                .collect(),
            changes: vec![],
            steady_state: None,
            steady_state_after: None,
            before_events: false,
            seed: 0,
            sensitivities: vec![],
            local_errors: false,
        })
        .collect())
}

/// The best rate of the points over the rounds, in points per second.
fn best(rounds: usize, mut round: impl FnMut() -> usize) -> f64 {
    (0..rounds)
        .map(|_| {
            let start = Instant::now();
            let points = round();
            points as f64 / start.elapsed().as_secs_f64()
        })
        .fold(0.0, f64::max)
}

fn rate(model: &Arc<LoadedModel>, requests: &[SimulationRequest], args: &Args, k: usize) -> f64 {
    match args.mode.as_str() {
        "batch" => {
            let workers = Workers::new(model.clone(), k).expect("a pool");
            // the instances of the threads, made before the rounds are timed
            let warm = workers.simulate(requests);
            if let Some(Err(failure)) = warm.iter().find(|outcome| outcome.is_err()) {
                panic!("a point failed: {failure}");
            }
            let slice = if args.slice == 0 {
                requests.len()
            } else {
                args.slice * k
            };
            best(args.rounds, || {
                requests
                    .chunks(slice)
                    .map(|slice| workers.simulate(slice).len())
                    .sum()
            })
        }
        "fed" => {
            let workers = Workers::new(model.clone(), k).expect("a pool");
            let run = || {
                workers
                    .map_fed(
                        requests.len(),
                        |range| Ok::<_, ()>(requests[range].to_vec()),
                        |instance, request| instance.simulate(request),
                        std::time::Duration::from_millis(20),
                        || Ok(()),
                    )
                    .expect("a batch nothing stopped")
                    .len()
            };
            run();
            best(args.rounds, run)
        }
        "pools" => {
            let pools: Vec<Workers> = (0..k)
                .map(|_| Workers::new(model.clone(), 1).expect("a pool"))
                .collect();
            let barrier = Barrier::new(k);
            let round = || {
                std::thread::scope(|scope| {
                    let handles: Vec<_> = pools
                        .iter()
                        .map(|workers| {
                            let barrier = &barrier;
                            scope.spawn(move || {
                                barrier.wait();
                                workers.simulate(requests).len()
                            })
                        })
                        .collect();
                    handles.into_iter().map(|h| h.join().unwrap()).sum()
                })
            };
            if let Some(Err(failure)) = pools[0].simulate(&requests[..1]).pop() {
                panic!("a point failed: {failure}");
            }
            round();
            best(args.rounds, round)
        }
        mode => panic!("no mode {mode}"),
    }
}

fn main() -> anyhow::Result<()> {
    let args = args()?;
    let sbml = std::fs::read_to_string(&args.model)?;
    let compiled = compile(&sbml, &CompileOptions::default())?;
    let runtime = Runtime::embedded()?;
    let model = Arc::new(runtime.load(&compiled.wasm, &compiled.description)?);
    let requests = requests(&model, &args)?;

    let mut one = None;
    for &k in &args.threads {
        let rate = rate(&model, &requests, &args, k);
        let one = *one.get_or_insert(rate / k as f64);
        println!(
            "{} threads={k:>2} points/s={rate:>9.0} efficiency={:>5.1} %",
            args.mode,
            100.0 * rate / (k as f64 * one)
        );
    }
    Ok(())
}
