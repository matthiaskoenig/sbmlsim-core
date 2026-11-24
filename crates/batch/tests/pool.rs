//! The pool of a batch: the number of its threads, the order of its points, the bits of a
//! serial run, the instance of a thread and the failures which stay with their own point.
//!
//! The model is the decay model `y' = -k y` of `crates/host/tests/decay-model`, which the
//! tests of the host and of the command line simulate as well.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use sbmlsim_abi::{EngineError, ModelDescription, SimulationRequest, SimulationResult};
use sbmlsim_batch::{
    BatchError, MAX_THREADS, Plan, PointFailure, WorkerPool, Workers, thread_count,
};
use sbmlsim_host::{LoadedModel, Runtime};

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

fn model() -> Arc<LoadedModel> {
    let wasm = wat::parse_str(include_str!("../../host/tests/decay.wat")).unwrap();
    let desc: ModelDescription =
        serde_json::from_str(include_str!("../../host/tests/decay-model/model.json")).unwrap();
    Arc::new(runtime().load(&wasm, &desc).unwrap())
}

fn request() -> SimulationRequest {
    serde_json::from_str(include_str!("../../host/tests/decay-model/request.json")).unwrap()
}

/// The requests of a batch, which differ in their end, so that a point is told by its table.
fn requests(ends: std::ops::RangeInclusive<u32>) -> Vec<SimulationRequest> {
    ends.map(|end| SimulationRequest {
        end: f64::from(end),
        ..request()
    })
    .collect()
}

/// The value of a column in the last row of a table.
fn last(table: &SimulationResult, column: &str) -> f64 {
    let at = table
        .columns
        .iter()
        .position(|name| name == column)
        .unwrap_or_else(|| panic!("no column {column} in {:?}", table.columns));
    table.values[(table.n_rows - 1) * table.columns.len() + at]
}

#[test]
fn a_thread_count_outside_the_bounds_is_refused() {
    for threads in [0, -1, MAX_THREADS as i64 + 1] {
        assert!(
            matches!(thread_count(Some(threads)), Err(BatchError::Threads(_))),
            "{threads} is not refused"
        );
    }
}

#[test]
fn a_thread_count_within_the_bounds_is_the_number_given() {
    assert_eq!(thread_count(Some(4)).unwrap(), 4);
}

#[test]
fn the_thread_count_of_no_number_is_within_the_bounds() {
    let threads = thread_count(None).unwrap();
    assert!((1..=MAX_THREADS).contains(&threads), "{threads}");
}

#[test]
fn a_pool_has_the_threads_it_was_made_with() {
    assert_eq!(Workers::new(model(), 3).unwrap().threads(), 3);
}

/// A batch of `requests` fed in the ranges `map_fed` asks for, which records the ranges.
fn fed(
    workers: &Workers,
    requests: &[SimulationRequest],
    ranges: &mut Vec<std::ops::Range<usize>>,
) -> Result<Vec<f64>, String> {
    let points = workers.map_fed(
        requests.len(),
        |range| {
            ranges.push(range.clone());
            Ok::<_, String>(requests[range].to_vec())
        },
        |instance, request| instance.simulate(request),
        Duration::from_millis(1),
        || Ok(()),
    )?;
    Ok(points
        .iter()
        .map(|point| last(point.as_ref().unwrap(), "time"))
        .collect())
}

#[test]
fn a_fed_batch_gives_the_points_in_their_order() {
    let workers = Workers::new(model(), 4).unwrap();
    let mut ranges = Vec::new();
    let ends = fed(&workers, &requests(1..=50), &mut ranges).unwrap();
    assert_eq!(ends, (1..=50).map(f64::from).collect::<Vec<f64>>());
    // as many items as the pool has threads first, then twice the range before
    assert_eq!(ranges, [0..4, 4..12, 12..28, 28..50]);
}

#[test]
fn a_fed_batch_without_points_reads_and_asks_nothing() {
    let workers = Workers::new(model(), 2).unwrap();
    let points = workers.map_fed(
        0,
        |_| -> Result<Vec<SimulationRequest>, String> { panic!("fed a batch of no point") },
        |instance, request| instance.simulate(request),
        Duration::ZERO,
        || panic!("watched a batch of no point"),
    );
    assert!(points.unwrap().is_empty());
    assert_eq!(workers.instances_made(), 0);
}

#[test]
fn an_error_of_the_feed_stops_the_batch() {
    let workers = Workers::new(model(), 2).unwrap();
    let requests = requests(1..=20);
    let ran = AtomicUsize::new(0);
    let points = workers.map_fed(
        requests.len(),
        |range| {
            if range.start > 0 {
                return Err(format!("no items at {}", range.start));
            }
            Ok(requests[range].to_vec())
        },
        |instance, request| {
            ran.fetch_add(1, Ordering::SeqCst);
            instance.simulate(request)
        },
        Duration::from_millis(1),
        || Ok(()),
    );
    assert_eq!(points.unwrap_err(), "no items at 2");
    // the first range was read before the error and may have started
    assert!(ran.load(Ordering::SeqCst) <= 2);
}

#[test]
fn a_stopped_batch_runs_no_point_after_the_stop() {
    let workers = Workers::new(model(), 2).unwrap();
    let requests = requests(1..=64);
    let (started, ran) = (AtomicBool::new(false), AtomicUsize::new(0));
    let points = workers.map_fed(
        requests.len(),
        |range| Ok::<_, &str>(requests[range].to_vec()),
        |instance, request| {
            started.store(true, Ordering::SeqCst);
            // every point is long enough for the watch of the caller to stop the batch
            // before the last one starts
            std::thread::sleep(Duration::from_millis(5));
            ran.fetch_add(1, Ordering::SeqCst);
            instance.simulate(request)
        },
        Duration::from_millis(1),
        || match started.load(Ordering::SeqCst) {
            true => Err("stopped"),
            false => Ok(()),
        },
    );
    assert_eq!(points.unwrap_err(), "stopped");
    // what ran is final once the batch answers: no thread of the pool works on after it
    let after = ran.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(ran.load(Ordering::SeqCst), after);
    assert!(
        after < requests.len(),
        "{after} points of a stopped batch ran"
    );
}

#[test]
fn a_pool_of_no_thread_is_refused() {
    assert!(matches!(
        Workers::new(model(), 0),
        Err(BatchError::Threads(0))
    ));
}

#[test]
fn the_points_keep_the_order_of_their_requests() {
    let workers = Workers::new(model(), 4).unwrap();
    let requests = requests(1..=12);
    let points = workers.simulate(&requests);
    let ends: Vec<f64> = points
        .iter()
        .map(|point| last(point.as_ref().unwrap(), "time"))
        .collect();
    assert_eq!(ends, (1..=12).map(f64::from).collect::<Vec<f64>>());
}

#[test]
fn a_batch_on_many_threads_gives_the_bits_of_one_thread() {
    let requests = requests(1..=20);
    let one = Workers::new(model(), 1).unwrap().simulate(&requests);
    let many = Workers::new(model(), 8).unwrap().simulate(&requests);
    for (at, (serial, parallel)) in one.iter().zip(&many).enumerate() {
        let (serial, parallel) = (serial.as_ref().unwrap(), parallel.as_ref().unwrap());
        assert_eq!(serial.columns, parallel.columns, "point {at}");
        assert!(
            serial
                .values
                .iter()
                .zip(&parallel.values)
                .all(|(x, y)| x.to_bits() == y.to_bits()),
            "point {at} differs in its bits"
        );
    }
}

#[test]
fn the_threads_reuse_their_instances_across_batches() {
    let workers = Workers::new(model(), 2).unwrap();
    // Which thread takes a point is rayon's to decide, so a batch of four points may run on one
    // thread or on both, and a later batch may be the first to reach a thread, which then makes
    // its instance: the count of a pool which reuses them rises to the number of its threads and
    // stops there. What the pool promises is an instance per thread and not one per batch, so
    // sixteen points over four batches make at most the two of its two threads, and every one
    // which was made is still held, i.e. none was dropped and made again.
    for batch in 1..=4 {
        workers.simulate(&requests(1..=4));
        let made = workers.instances_made();
        assert!(
            (1..=2).contains(&made),
            "{made} instances of two threads after batch {batch}"
        );
        assert_eq!(made, workers.instances_held(), "after batch {batch}");
    }
}

#[test]
fn a_batch_without_points_makes_no_instance() {
    let workers = Workers::new(model(), 2).unwrap();
    assert!(workers.simulate(&[]).is_empty());
    assert_eq!(workers.instances_made(), 0);
}

#[test]
fn a_point_which_fails_leaves_the_points_around_it_their_results() {
    let workers = Workers::new(model(), 2).unwrap();
    let mut unknown = request();
    unknown.selections = vec!["time".to_string(), "nope".to_string()];
    let requests = vec![request(), unknown, request()];
    let points = workers.simulate(&requests);
    assert!(points[0].is_ok(), "{:?}", points[0]);
    assert!(
        matches!(
            &points[1],
            Err(PointFailure::Engine(EngineError::Request(_)))
        ),
        "{:?}",
        points[1]
    );
    assert!(points[2].is_ok(), "{:?}", points[2]);
}

#[test]
fn a_point_which_panics_fails_alone() {
    let workers = Workers::new(model(), 2).unwrap();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let points = workers.map(&[0_usize, 1, 2], |_instance, &item| {
        assert_ne!(item, 1, "the point of the test panics");
        Ok::<usize, EngineError>(item)
    });
    std::panic::set_hook(hook);
    assert_eq!(points[0].as_ref().ok(), Some(&0));
    match &points[1] {
        Err(PointFailure::Internal(message)) => {
            assert!(message.contains("panicked"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(points[2].as_ref().ok(), Some(&2));
}

#[test]
fn an_instance_which_fails_internally_is_not_used_again() {
    let workers = Workers::new(model(), 1).unwrap();
    let failed = AtomicBool::new(false);
    let points = workers.map(&[0_usize, 1], |_instance, &item| {
        if failed.swap(true, Ordering::SeqCst) {
            Ok(item)
        } else {
            Err(EngineError::Internal("the engine trapped".into()))
        }
    });
    assert_eq!(points.iter().filter(|point| point.is_ok()).count(), 1);
    assert_eq!(workers.instances_made(), 2);
    assert_eq!(workers.instances_held(), 1);
}

#[test]
fn a_pool_which_grows_keeps_the_instances_it_has() {
    let mut pool = WorkerPool::new(model(), 8);
    pool.ensure(1).unwrap();
    let workers = pool.workers().unwrap();
    assert_eq!(workers.threads(), 1);
    workers.simulate(&requests(1..=1));
    assert_eq!(workers.instances_held(), 1);
    pool.ensure(4).unwrap();
    let workers = pool.workers().unwrap();
    assert_eq!(workers.threads(), 4);
    assert_eq!(workers.instances_held(), 1);
    assert_eq!(workers.instances_made(), 1);
}

#[test]
fn a_pool_does_not_shrink_for_a_smaller_batch() {
    let mut pool = WorkerPool::new(model(), 8);
    pool.ensure(4).unwrap();
    pool.ensure(2).unwrap();
    assert_eq!(pool.workers().unwrap().threads(), 4);
}

#[test]
fn a_pool_which_is_taken_leaves_no_workers() {
    let mut pool = WorkerPool::new(model(), 2);
    pool.ensure(2).unwrap();
    assert!(pool.take().is_some());
    assert!(pool.workers().is_none());
}

#[test]
fn the_points_of_a_prepared_batch_take_their_values() {
    let model = model();
    let prepared = model.prepare(&request(), &["k".to_string()]).unwrap();
    let workers = Workers::new(model, 2).unwrap();
    let points = workers.simulate_prepared(&prepared, &[vec![0.1], vec![1.0]]);
    let slow = last(points[0].as_ref().unwrap(), "y");
    let fast = last(points[1].as_ref().unwrap(), "y");
    assert!(fast < slow, "y of k=1.0 is {fast}, of k=0.1 it is {slow}");
}

#[test]
fn a_pool_knows_the_model_of_its_batches() {
    let model = model();
    let pool = WorkerPool::new(model.clone(), 2);
    assert!(Arc::ptr_eq(pool.model(), &model));
}

#[test]
fn the_plans_of_a_batch_run_their_own_requests() {
    let model = model();
    let varying = ["k".to_string()];
    let short = model
        .prepare(
            &SimulationRequest {
                end: 2.0,
                ..request()
            },
            &varying,
        )
        .unwrap();
    let long = model
        .prepare(
            &SimulationRequest {
                end: 8.0,
                ..request()
            },
            &varying,
        )
        .unwrap();
    let workers = Workers::new(model, 2).unwrap();
    let plans = [
        Plan {
            prepared: short.clone(),
            values: vec![1.0],
        },
        Plan {
            prepared: long,
            values: vec![0.5],
        },
        Plan {
            prepared: short,
            values: vec![0.2],
        },
    ];
    let points = workers.simulate_plans(&plans);
    let ends: Vec<f64> = points
        .iter()
        .map(|point| last(point.as_ref().unwrap(), "time"))
        .collect();
    assert_eq!(ends, vec![2.0, 8.0, 2.0]);
    // the same request at the larger k has decayed further at its end
    let (fast, slow) = (
        last(points[0].as_ref().unwrap(), "y"),
        last(points[2].as_ref().unwrap(), "y"),
    );
    assert!(fast < slow, "y of k=1.0 is {fast}, of k=0.2 it is {slow}");
}

#[test]
fn a_plan_of_another_model_fails_its_own_point() {
    let prepared = model().prepare(&request(), &[]).unwrap();
    let workers = Workers::new(model(), 1).unwrap();
    let points = workers.simulate_plans(&[
        Plan {
            prepared,
            values: vec![],
        },
        Plan {
            prepared: workers.model().prepare(&request(), &[]).unwrap(),
            values: vec![],
        },
    ]);
    assert!(
        matches!(
            &points[0],
            Err(PointFailure::Engine(EngineError::Request(_)))
        ),
        "{:?}",
        points[0]
    );
    assert!(points[1].is_ok(), "{:?}", points[1]);
}
