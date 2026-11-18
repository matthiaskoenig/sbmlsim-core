//! The batch of simulations over the threads of a pool, each on an engine instance of its own.
//!
//! A scan, the points of a finite difference Jacobian and the population of an optimizer are
//! the same work: many simulations of one model which do not depend on each other.
//! [`Workers`] is the pool which runs them. Every thread of it simulates on an instance of its
//! own, made at the first point of the thread and reused by every point after it, so the
//! results of a batch are those of a serial run bitwise, whatever the threads are, and the
//! cost of an instance (0.33 ms for a model of 700 kB of description) is paid once per thread
//! instead of once per point. A point which fails, and a point which panics, fails alone and
//! leaves the points around it their results.
//!
//! [`WorkerPool`] keeps such a pool and its instances across batches and grows it to the
//! points a batch has, which is what a caller of many small batches needs: an optimizer which
//! evaluates one objective after the other, or a host which runs the scans of a session.

use std::any::Any;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rayon::prelude::*;
use sbmlsim_abi::{EngineError, SimulationRequest, SimulationResult};
use sbmlsim_host::{LoadedModel, NativeInstance, PreparedSimulation};

/// How many points of a batch one thread of the pool gets per slice ([`Workers::slice_len`]).
pub const SLICE_PER_THREAD: usize = 8;

/// The most threads a batch runs on. More of them is no use for simulations, which are CPU
/// work alone, and a thread costs a stack and an instance, so a number nothing bounds is an
/// allocation the process would be killed for rather than an error it answers with.
pub const MAX_THREADS: usize = 1024;

/// A batch which cannot run.
#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    /// A number of threads which is none: fewer than one or more than [`MAX_THREADS`].
    #[error("threads must be at least 1 and at most {MAX_THREADS}, not {0}")]
    Threads(i64),
    /// A pool of threads which does not start.
    #[error("no pool of {threads} threads: {source}")]
    Pool {
        /// The threads the pool was asked for.
        threads: usize,
        /// Why rayon did not build it.
        #[source]
        source: rayon::ThreadPoolBuildError,
    },
}

/// Why a point of a batch failed: the error of the engine, or a failure of the host around the
/// point (no instance of its own, or a panic).
#[derive(Debug, thiserror::Error)]
pub enum PointFailure {
    /// The error the engine answered the point with.
    #[error(transparent)]
    Engine(EngineError),
    /// A failure of the host around the point, which is a defect.
    #[error("{0}")]
    Internal(String),
}

/// One point of a batch: what its work gave, or the failure which ended it. A point is never
/// the end of its batch, so every point of a batch has an outcome of its own.
pub type Outcome<T> = Result<T, PointFailure>;

/// The number of threads of a batch: the given one, or every CPU of the process without it.
///
/// It is the number a caller takes from its own interface, where a number is a number of a
/// request and not of the code: fewer than one thread and more than [`MAX_THREADS`] are
/// refused with [`BatchError::Threads`].
pub fn thread_count(threads: Option<i64>) -> Result<usize, BatchError> {
    let Some(threads) = threads else {
        return Ok(std::thread::available_parallelism()
            .map_or(1, NonZeroUsize::get)
            .min(MAX_THREADS));
    };
    usize::try_from(threads)
        .ok()
        .filter(|threads| (1..=MAX_THREADS).contains(threads))
        .ok_or(BatchError::Threads(threads))
}

/// One point of a batch of plans: a prepared request and the values of its varying targets.
///
/// A batch of plans is what one evaluation of an objective is: one request per condition of a
/// fit, every one of them at the same parameters, and the points of a finite difference
/// Jacobian are that batch once per parameter. The plans of such a batch are all of it, so a
/// fit of a model of many conditions runs its whole Jacobian on every thread of the pool
/// instead of one condition after the other.
pub struct Plan {
    /// The request, which any number of plans of a batch share: its plan is resolved once per
    /// instance, so the points after the first transfer their values and nothing else.
    pub prepared: PreparedSimulation,
    /// The values of the varying targets of the request, in the order they were prepared in.
    pub values: Vec<f64>,
}

/// The instance a thread of the pool simulates on, taken out of its slot while it simulates.
type Slot = Mutex<Option<NativeInstance>>;

/// The instances of the threads of a pool, which outlive a pool the batches grow.
struct Instances {
    slots: Vec<Slot>,
    made: AtomicUsize,
}

/// The message of the payload of a panic.
fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic of no message")
}

/// A pool of `threads` threads, or why it does not start.
fn pool_of(threads: usize) -> Result<rayon::ThreadPool, BatchError> {
    if !(1..=MAX_THREADS).contains(&threads) {
        return Err(BatchError::Threads(
            i64::try_from(threads).unwrap_or(i64::MAX),
        ));
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|source| BatchError::Pool { threads, source })
}

/// A pool of threads with an engine instance per thread, which runs the points of a batch.
pub struct Workers {
    model: Arc<LoadedModel>,
    pool: rayon::ThreadPool,
    instances: Instances,
}

impl Workers {
    /// A pool of `threads` threads on one loaded model, without an instance yet: a thread
    /// makes its own at its first point.
    pub fn new(model: Arc<LoadedModel>, threads: usize) -> Result<Workers, BatchError> {
        Ok(Workers::of(model, pool_of(threads)?, threads))
    }

    /// A pool with fresh slots for its threads.
    fn of(model: Arc<LoadedModel>, pool: rayon::ThreadPool, threads: usize) -> Workers {
        Workers {
            model,
            pool,
            instances: Instances {
                slots: (0..threads).map(|_| Mutex::new(None)).collect(),
                made: AtomicUsize::new(0),
            },
        }
    }

    /// This pool on another pool of threads, which keeps the instances the threads of this one
    /// made; `threads` is at least the threads of this pool, so no instance is dropped.
    fn with_pool(mut self, pool: rayon::ThreadPool, threads: usize) -> Workers {
        self.instances
            .slots
            .resize_with(threads, || Mutex::new(None));
        Workers {
            model: self.model,
            pool,
            instances: self.instances,
        }
    }

    /// The model every point of a batch simulates.
    pub fn model(&self) -> &Arc<LoadedModel> {
        &self.model
    }

    /// The threads of the pool, which is the parallelism of a batch.
    pub fn threads(&self) -> usize {
        self.instances.slots.len()
    }

    /// How many points a caller which runs a batch in slices gives one slice, so that the work
    /// of the threads is spread and a long batch is still interrupted between two slices.
    pub fn slice_len(&self) -> usize {
        SLICE_PER_THREAD * self.threads()
    }

    /// How many instances the pool has made, which is one per thread of a batch which ran and
    /// one more per instance an internal error or a panic dropped.
    pub fn instances_made(&self) -> usize {
        self.instances.made.load(Ordering::Relaxed)
    }

    /// How many instances the pool holds for the points of the batches after this one.
    pub fn instances_held(&self) -> usize {
        self.instances
            .slots
            .iter()
            .filter(|slot| {
                slot.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .is_some()
            })
            .count()
    }

    /// Runs `work` on every item on the instance of the thread which runs it and gives one
    /// outcome per item, in the order of the items.
    ///
    /// `work` is the whole work of a point and runs on a thread of the pool, so a caller whose
    /// point is more than a simulation (the residuals of an objective, the rows of a table)
    /// does that work there as well instead of after the batch. An error of the engine fails
    /// its own point; an internal error, which an instance never recovers from, drops the
    /// instance of its thread, and the next point of that thread makes a fresh one. A point
    /// which panics fails alone with [`PointFailure::Internal`].
    ///
    /// A batch without items runs nothing and makes no instance.
    pub fn map<T, R, F>(&self, items: &[T], work: F) -> Vec<Outcome<R>>
    where
        T: Sync,
        R: Send,
        F: Fn(&mut NativeInstance, &T) -> Result<R, EngineError> + Sync,
    {
        if items.is_empty() {
            return Vec::new();
        }
        self.pool.install(|| {
            items
                .par_iter()
                .map(|item| self.point(&work, item))
                .collect()
        })
    }

    /// Runs `work` on the pool of the batch and gives what it answers.
    ///
    /// An optimizer which spreads work of its own over rayon (the population of a generation
    /// of a differential evolution) spreads it over the threads of the pool this way, and not
    /// over the threads of the machine, which the global pool of rayon has. A batch inside
    /// `work` runs on the pool as well: the instance of a point is taken while the point
    /// simulates, so a point of the inner batch takes the instance of the thread it lands on
    /// and no two points share one.
    pub fn install<R: Send>(&self, work: impl FnOnce() -> R + Send) -> R {
        self.pool.install(work)
    }

    /// Simulates every request of a batch and gives one table per request, in their order.
    pub fn simulate(&self, requests: &[SimulationRequest]) -> Vec<Outcome<SimulationResult>> {
        self.map(requests, |instance, request| instance.simulate(request))
    }

    /// Simulates a prepared request at every point of a batch, which are the values of the
    /// varying targets of the request, and gives one table per point, in their order.
    ///
    /// The plan of the request is resolved once per instance, so a batch of many points on a
    /// pool which is kept transfers a vector of values per point and nothing else.
    pub fn simulate_prepared(
        &self,
        prepared: &PreparedSimulation,
        points: &[Vec<f64>],
    ) -> Vec<Outcome<SimulationResult>> {
        self.map(points, |instance, values| {
            instance.simulate_prepared(prepared, values)
        })
    }

    /// Simulates every plan of a batch, which are requests of their own, and gives one table
    /// per plan, in their order.
    ///
    /// A plan of another loaded model fails its own point, as every other error of a request
    /// does, and leaves the plans around it their results.
    pub fn simulate_plans(&self, plans: &[Plan]) -> Vec<Outcome<SimulationResult>> {
        self.map(plans, |instance, plan| {
            instance.simulate_prepared(&plan.prepared, &plan.values)
        })
    }

    /// One point of a batch, whose panic fails the point and no other one: the instance of the
    /// point is dropped while it unwinds, so its thread gets a fresh one for its next point
    /// and the poisoned lock of its slot is read as the empty slot it is.
    fn point<T, R, F>(&self, work: &F, item: &T) -> Outcome<R>
    where
        F: Fn(&mut NativeInstance, &T) -> Result<R, EngineError> + Sync,
    {
        catch_unwind(AssertUnwindSafe(|| {
            self.on_instance(|instance| work(instance, item))
        }))
        .unwrap_or_else(|payload| {
            Err(PointFailure::Internal(format!(
                "the point panicked: {}",
                panic_message(&*payload)
            )))
        })
    }

    /// Runs the work of a point on the instance of the thread of the pool which runs it, which
    /// it takes out of its slot while it works and puts back for the points after it.
    fn on_instance<R>(
        &self,
        work: impl FnOnce(&mut NativeInstance) -> Result<R, EngineError>,
    ) -> Outcome<R> {
        // the slot of the thread which runs the point, which no other thread reads, so that
        // its lock is never contended; a point outside of the pool, which rayon does not run,
        // and one whose slot a panic left empty work on an instance of their own
        let slot =
            rayon::current_thread_index().and_then(|thread| self.instances.slots.get(thread));
        let mut slot = slot.map(|slot| slot.lock().unwrap_or_else(PoisonError::into_inner));
        let mut instance = match slot.as_mut().and_then(|slot| slot.take()) {
            Some(instance) => instance,
            None => match self.model.instantiate_native() {
                Ok(instance) => {
                    self.instances.made.fetch_add(1, Ordering::Relaxed);
                    instance
                }
                Err(error) => {
                    return Err(PointFailure::Internal(format!(
                        "the point has no instance: {error:#}"
                    )));
                }
            },
        };
        let result = work(&mut instance);
        // an instance which trapped refuses every request after it (`EngineError::Internal`),
        // so it is dropped here and the thread gets a fresh one for its next point
        if let Some(slot) = slot.as_mut()
            && !matches!(result, Err(EngineError::Internal(_)))
        {
            **slot = Some(instance);
        }
        result.map_err(PointFailure::Engine)
    }
}

/// A pool which is kept across batches, bounded by the threads it was made with and grown to
/// the points of a batch: a batch of three points runs on three threads, and a batch of a
/// hundred on the bound. It never shrinks, so the instances of its threads are made once.
pub struct WorkerPool {
    model: Arc<LoadedModel>,
    threads: usize,
    workers: Option<Workers>,
}

impl WorkerPool {
    /// A pool of at most `threads` threads on one loaded model, which starts no thread and
    /// makes no instance before its first batch.
    pub fn new(model: Arc<LoadedModel>, threads: usize) -> WorkerPool {
        WorkerPool {
            model,
            threads,
            workers: None,
        }
    }

    /// The threads the pool grows to at most.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// The model every batch of the pool simulates, which a caller compares with the model of
    /// a prepared request before it runs the request on this pool.
    pub fn model(&self) -> &Arc<LoadedModel> {
        &self.model
    }

    /// Grows the pool to the threads a batch of `points` points runs on, which is `points`
    /// threads or the bound of the pool, whichever is smaller. A pool which is large enough
    /// already keeps the threads and the instances it has.
    ///
    /// A thread without a point of its own is a thread started for nothing, and rayon's pool
    /// does not wait for its threads when it is dropped, so a caller of many small batches
    /// would hold more threads than it asked for.
    pub fn ensure(&mut self, points: usize) -> Result<(), BatchError> {
        let threads = self.threads.min(points).max(1);
        if self
            .workers
            .as_ref()
            .is_none_or(|workers| workers.threads() < threads)
        {
            // the pool of the threads before the instances of the pool are moved into it, so
            // that a pool which does not start leaves the workers of the batches before it
            let pool = pool_of(threads)?;
            self.workers = Some(match self.workers.take() {
                Some(workers) => workers.with_pool(pool, threads),
                None => Workers::of(self.model.clone(), pool, threads),
            });
        }
        Ok(())
    }

    /// The workers of the pool, once a batch has grown it to its points.
    pub fn workers(&self) -> Option<&Workers> {
        self.workers.as_ref()
    }

    /// Takes the workers out of the pool, which a caller drops where dropping them costs
    /// nothing it holds. The pool makes new ones for the batch after it.
    pub fn take(&mut self) -> Option<Workers> {
        self.workers.take()
    }
}
