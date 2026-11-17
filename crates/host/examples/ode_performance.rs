//! Measures warmed ODE simulations, or compares two WASM engines in paired blocks.

use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use sbmlsim_abi::{ModelDescription, SimulationRequest, SimulationResult, SimulationStats};
use sbmlsim_host::{Instance, ModuleCache, NativeInstance, PreparedSimulation, Runtime};
use serde::{Deserialize, Serialize};

const USAGE: &str = "ode_performance JOBS.json
ode_performance paired JOBS.json BASELINE_RUNTIME FEATURE_RUNTIME
ode_performance runtime OUTPUT_DIR
ode_performance cold";

#[derive(Deserialize)]
struct Job {
    name: String,
    directory: PathBuf,
    request: SimulationRequest,
    native: bool,
    prepared: bool,
}

#[derive(Serialize)]
struct Measurement {
    name: String,
    samples_s: Vec<f64>,
    wall_samples_s: Vec<f64>,
    iterations: usize,
    result_hash: String,
    stats: [u64; SimulationStats::WORDS],
}

#[derive(Serialize)]
struct PairedMeasurement {
    name: String,
    baseline_samples_s: Vec<f64>,
    feature_samples_s: Vec<f64>,
    ratio: f64,
    iterations: usize,
}

// Keep both instances unboxed to match the benchmark setup. They are created outside the samples.
#[allow(clippy::large_enum_variant)]
enum Runner {
    Wasm(Instance),
    Native(NativeInstance),
}

impl Runner {
    fn simulate(
        &mut self,
        request: &SimulationRequest,
        plan: Option<&PreparedSimulation>,
    ) -> Result<SimulationResult, sbmlsim_abi::EngineError> {
        match (self, plan) {
            (Self::Wasm(instance), Some(plan)) => instance.simulate_prepared(plan, &[]),
            (Self::Wasm(instance), None) => instance.simulate(request),
            (Self::Native(instance), Some(plan)) => instance.simulate_prepared(plan, &[]),
            (Self::Native(instance), None) => instance.simulate(request),
        }
    }

    fn stats(&mut self) -> Result<SimulationStats> {
        Ok(match self {
            Self::Wasm(instance) => instance.stats()?,
            Self::Native(instance) => instance.stats(),
        })
    }
}

// CLOCK_THREAD_CPUTIME_ID on 64-bit Linux.
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
fn cpu_time() -> f64 {
    #[repr(C)]
    struct Timespec {
        seconds: i64,
        nanoseconds: i64,
    }

    unsafe extern "C" {
        fn clock_gettime(clock: i32, time: *mut Timespec) -> i32;
    }

    let mut time = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    // SAFETY: the pointer is writable and Timespec has the layout of the 64-bit Linux timespec.
    assert_eq!(unsafe { clock_gettime(3, &mut time) }, 0);
    time.seconds as f64 + time.nanoseconds as f64 * 1e-9
}

// The sample clock falls back to monotonic elapsed time on other systems.
#[cfg(not(all(target_os = "linux", target_pointer_width = "64")))]
fn cpu_time() -> f64 {
    static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64()
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("reading {}", path.display()))
}

fn jobs(path: &Path) -> Result<Vec<Job>> {
    let mut jobs: Vec<Job> = serde_json::from_slice(&read(path)?)
        .with_context(|| format!("parsing {}", path.display()))?;
    if let Some(parent) = path.parent() {
        for job in &mut jobs {
            job.directory = parent.join(&job.directory);
        }
    }
    Ok(jobs)
}

fn model(job: &Job) -> Result<(Vec<u8>, ModelDescription)> {
    let wasm = read(&job.directory.join("model.wasm"))?;
    let description = job.directory.join("model.json");
    let description = serde_json::from_slice(&read(&description)?)
        .with_context(|| format!("parsing {}", description.display()))?;
    Ok((wasm, description))
}

fn cache() -> ModuleCache {
    std::env::var_os("SBMLSIM_ODE_BENCH_CACHE")
        .map(PathBuf::from)
        .map_or(ModuleCache::Default, ModuleCache::Directory)
}

fn runtime(directory: &Path) -> Result<Runtime> {
    Runtime::new_with_cache(
        &read(&directory.join("engine.wasm"))?,
        &read(&directory.join("mathlib.wasm"))?,
        cache(),
    )
}

fn median_sample(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

fn run_batch(
    runner: &mut Runner,
    request: &SimulationRequest,
    plan: Option<&PreparedSimulation>,
    iterations: usize,
) -> Result<()> {
    for _ in 0..iterations {
        black_box(runner.simulate(request, plan)?);
    }
    Ok(())
}

fn iterations(
    runner: &mut Runner,
    request: &SimulationRequest,
    plan: Option<&PreparedSimulation>,
) -> Result<usize> {
    let start = cpu_time();
    run_batch(runner, request, plan, 16)?;
    let estimate = (cpu_time() - start) / 16.0;
    Ok(((0.03 / estimate) as usize).clamp(8, 100_000))
}

fn measure(job_file: &Path) -> Result<Vec<Measurement>> {
    let runtime = Runtime::embedded_with_cache(cache())?;
    let mut results = Vec::new();
    for job in jobs(job_file)? {
        let (wasm, description) = model(&job)?;
        let loaded = runtime.load(&wasm, &description)?;
        let plan = job
            .prepared
            .then(|| loaded.prepare(&job.request, &[]))
            .transpose()?;
        let mut runner = if job.native {
            Runner::Native(loaded.instantiate_native()?)
        } else {
            Runner::Wasm(loaded.instantiate()?)
        };
        let initial = runner.simulate(&job.request, plan.as_ref())?;
        let result_hash = initial
            .values
            .iter()
            .fold(0xcbf29ce484222325u64, |hash, value| {
                (hash ^ value.to_bits()).wrapping_mul(0x100000001b3)
            });
        run_batch(&mut runner, &job.request, plan.as_ref(), 10)?;
        let iterations = iterations(&mut runner, &job.request, plan.as_ref())?;
        let mut samples_s = Vec::new();
        let mut wall_samples_s = Vec::new();
        for _ in 0..9 {
            let start = Instant::now();
            let cpu_start = cpu_time();
            run_batch(&mut runner, &job.request, plan.as_ref(), iterations)?;
            samples_s.push((cpu_time() - cpu_start) / iterations as f64);
            wall_samples_s.push(start.elapsed().as_secs_f64() / iterations as f64);
        }
        let stats = runner.stats()?.words();
        eprintln!("{}: {:.3} us", job.name, median_sample(&samples_s) * 1e6);
        results.push(Measurement {
            name: job.name,
            samples_s,
            wall_samples_s,
            iterations,
            result_hash: format!("{result_hash:016x}"),
            stats,
        });
    }
    Ok(results)
}

fn paired(job_file: &Path, baseline: &Path, feature: &Path) -> Result<Vec<PairedMeasurement>> {
    let blocks = std::env::var_os("SBMLSIM_ODE_BENCH_BLOCKS")
        .map(|value| value.to_string_lossy().parse::<usize>())
        .transpose()
        .context("SBMLSIM_ODE_BENCH_BLOCKS must be a positive integer")?
        .unwrap_or(9);
    ensure!(blocks > 0, "SBMLSIM_ODE_BENCH_BLOCKS must be positive");
    let baseline = runtime(baseline)?;
    let feature = runtime(feature)?;
    let mut results = Vec::new();
    for job in jobs(job_file)?.into_iter().filter(|job| !job.native) {
        let (wasm, description) = model(&job)?;
        let loaded_a = baseline.load(&wasm, &description)?;
        let loaded_b = feature.load(&wasm, &description)?;
        let plan_a = job
            .prepared
            .then(|| loaded_a.prepare(&job.request, &[]))
            .transpose()?;
        let plan_b = job
            .prepared
            .then(|| loaded_b.prepare(&job.request, &[]))
            .transpose()?;
        let mut a = Runner::Wasm(loaded_a.instantiate()?);
        let mut b = Runner::Wasm(loaded_b.instantiate()?);
        let output_a = a.simulate(&job.request, plan_a.as_ref())?;
        let output_b = b.simulate(&job.request, plan_b.as_ref())?;
        ensure!(
            output_a.columns == output_b.columns
                && output_a.n_rows == output_b.n_rows
                && output_a.values.len() == output_b.values.len()
                && output_a
                    .values
                    .iter()
                    .zip(&output_b.values)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
            "outputs differ for {}",
            job.name
        );
        ensure!(a.stats()? == b.stats()?, "counters differ for {}", job.name);

        for _ in 0..20 {
            run_batch(&mut a, &job.request, plan_a.as_ref(), 1)?;
            run_batch(&mut b, &job.request, plan_b.as_ref(), 1)?;
        }
        let iterations = iterations(&mut a, &job.request, plan_a.as_ref())?;
        let (mut baseline_samples_s, mut feature_samples_s) = (Vec::new(), Vec::new());
        for _ in 0..blocks {
            for is_feature in [false, true, true, false] {
                let (runner, plan, samples) = if is_feature {
                    (&mut b, plan_b.as_ref(), &mut feature_samples_s)
                } else {
                    (&mut a, plan_a.as_ref(), &mut baseline_samples_s)
                };
                let start = cpu_time();
                run_batch(runner, &job.request, plan, iterations)?;
                samples.push((cpu_time() - start) / iterations as f64);
            }
        }
        let ratio = median_sample(&feature_samples_s) / median_sample(&baseline_samples_s);
        eprintln!("{ratio:.4} {}", job.name);
        results.push(PairedMeasurement {
            name: job.name,
            baseline_samples_s,
            feature_samples_s,
            ratio,
            iterations,
        });
    }
    Ok(results)
}

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [mode, output] if mode == "runtime" => {
            let output = Path::new(output);
            fs::create_dir_all(output)?;
            fs::write(output.join("engine.wasm"), sbmlsim_host::ENGINE_WASM)?;
            fs::write(output.join("mathlib.wasm"), sbmlsim_host::MATHLIB_WASM)?;
        }
        [mode, input, baseline, feature] if mode == "paired" => {
            let results = paired(Path::new(input), Path::new(baseline), Path::new(feature))?;
            println!("{}", serde_json::to_string(&results)?);
        }
        [mode] if mode == "cold" => {
            let start = Instant::now();
            black_box(Runtime::embedded_with_cache(ModuleCache::Disabled)?);
            println!("{}", start.elapsed().as_secs_f64());
        }
        [input] if input != "paired" && input != "runtime" => {
            println!("{}", serde_json::to_string(&measure(Path::new(input))?)?);
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}
