//! The speed of the cell model of `testdata/models` on the QFunction target against the stepper
//! of the engine with the native model of the C target, on one core.
//!
//! ```text
//! cargo run --release -p sbmlsim-c --example qfunction_performance -- \
//!     [--points 1000] [--hours 24] [--rounds 5] [--rtol 1e-6] [--atol 1e-9]
//! ```
//!
//! The points are the 50 seeded parameter sets of the tests (`tests/common`), repeated, each
//! initialized by a stepper. A point-step is one interval of 60 min of one point, the unit of
//! work of both paths: the QFunction harness (`run_points`, in blocks of 1 and of 8 points, the
//! block of libCEED's blocked CPU backend) applies the QFunction until every point reaches the
//! end of the interval, the stepper advances each point over it. Both at the same tolerances;
//! the best of `--rounds` rounds. Pin the process to one core for a stable number
//! (`taskset -c 2`).

#[path = "../tests/common/mod.rs"]
mod common;

use std::sync::Arc;
use std::time::Instant;

use common::{cell_model, cell_parameter_sets};
use sbmlsim_abi::{IntegratorSettings, Tolerance};
use sbmlsim_c::{
    CModel, COUNTS, Library, Points, RosenbrockContext, generate, generate_qfunction, run_points,
};
use sbmlsim_engine::Stepper;

struct Args {
    points: usize,
    hours: usize,
    rounds: usize,
    rtol: f64,
    atol: f64,
}

fn args() -> Args {
    let mut args = Args {
        points: 1000,
        hours: 24,
        rounds: 5,
        rtol: 1e-6,
        atol: 1e-9,
    };
    let mut words = std::env::args().skip(1);
    while let Some(flag) = words.next() {
        let value = words
            .next()
            .unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--points" => args.points = value.parse().unwrap(),
            "--hours" => args.hours = value.parse().unwrap(),
            "--rounds" => args.rounds = value.parse().unwrap(),
            "--rtol" => args.rtol = value.parse().unwrap(),
            "--atol" => args.atol = value.parse().unwrap(),
            _ => panic!("unknown flag {flag}"),
        }
    }
    args
}

/// A line of the table: the path, point-steps per second, microseconds per point-step, and
/// the steps and the rejected attempts per point-step.
fn report(path: &str, point_steps: usize, seconds: f64, steps: f64, rejected: f64) {
    let n = point_steps as f64;
    println!(
        "| {path} | {:.0} | {:.2} | {:.2} | {:.3} |",
        n / seconds,
        1e6 * seconds / n,
        steps / n,
        rejected / n
    );
}

fn main() {
    let args = args();
    let sbml = cell_model();
    let source = generate(&sbml).unwrap();
    let model = CModel::new(Library::build(&source.code).unwrap(), &source.description).unwrap();
    let description = Arc::new(source.description);
    let settings = IntegratorSettings {
        relative_tolerance: args.rtol,
        absolute_tolerance: Tolerance::Scalar(args.atol),
        ..IntegratorSettings::default()
    };
    let sets = cell_parameter_sets();
    let mut steppers: Vec<Stepper> = (0..args.points)
        .map(|i| {
            Stepper::new(
                Box::new(model.clone()),
                description.clone(),
                settings.clone(),
                &sets[i % sets.len()],
            )
            .unwrap()
        })
        .collect();
    let initial: Vec<Vec<f64>> = steppers.iter().map(|s| s.state().to_vec()).collect();
    let parameters: Vec<f64> = steppers
        .iter()
        .flat_map(|s| s.parameters().to_vec())
        .collect();

    let q = generate_qfunction(&sbml).unwrap();
    let library = Library::build(&q.host_code()).unwrap();
    let start = Points::new(&q, &initial.concat(), &parameters).unwrap();
    let point_steps = args.points * args.hours;
    println!(
        "{} points of the cell model, {} intervals of 60 min, rtol {}, atol {}, best of {} rounds\n",
        args.points, args.hours, args.rtol, args.atol, args.rounds
    );
    println!("| path | point-steps/s | us/point-step | steps/point-step | rejected/point-step |");
    println!("|---|---:|---:|---:|---:|");
    for block in [1, 8] {
        let mut best = f64::INFINITY;
        let mut counts = [0.0; COUNTS];
        for _ in 0..args.rounds {
            let mut points = start.clone();
            let begin = Instant::now();
            for k in 0..args.hours {
                let context = RosenbrockContext {
                    t1: 60.0 * (k + 1) as f64,
                    rtol: args.rtol,
                    atol: args.atol,
                    h_min: 1e-10,
                };
                points.start(60.0 * k as f64);
                run_points(&library, &q, &context, &mut points, block).unwrap();
            }
            best = best.min(begin.elapsed().as_secs_f64());
            assert!(points.finished() && points.control.chunks(4).all(|c| c[3] == 0.0));
            for c in points.counts.chunks(COUNTS) {
                counts[0] += c[0] / args.rounds as f64;
                counts[1] += c[1] / args.rounds as f64;
            }
        }
        report(
            &format!("QFunction, RODAS4, block {block}"),
            point_steps,
            best,
            counts[0],
            counts[1],
        );
    }

    let mut best = f64::INFINITY;
    let (mut steps, mut rejected) = (0.0, 0.0);
    for _ in 0..args.rounds {
        let begin = Instant::now();
        for (stepper, y0) in steppers.iter_mut().zip(&initial) {
            let mut y = y0.clone();
            for k in 0..args.hours {
                let stats = stepper
                    .advance(&y, 60.0 * k as f64, 60.0 * (k + 1) as f64)
                    .unwrap();
                y.copy_from_slice(stepper.state());
                steps += stats.steps as f64 / args.rounds as f64;
                rejected += stats.rejected as f64 / args.rounds as f64;
            }
        }
        best = best.min(begin.elapsed().as_secs_f64());
    }
    report(
        "Stepper, CVODE BDF, CModel",
        point_steps,
        best,
        steps,
        rejected,
    );
}
