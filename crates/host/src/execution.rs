//! One epoch clock per runtime, shared by loaded models and their instances.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use wasmtime::{Engine, Store};

const TICK: Duration = Duration::from_millis(10);
pub const DEFAULT_EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);

struct Clock {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Clock {
    fn drop(&mut self) {
        let (stop, wake) = &*self.stop;
        *stop
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
pub(crate) struct Execution {
    // Keeping the clock alive also keeps the epoch-enabled engine interruptible when the
    // Runtime or LoadedModel that created an instance has already been dropped.
    _clock: Arc<Clock>,
    pub timeout: Option<Duration>,
}

impl Execution {
    pub fn new(engine: &Engine) -> std::io::Result<Self> {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let stopped = stop.clone();
        let engine = engine.clone();
        let thread = std::thread::Builder::new()
            .name("sbmlsim-epochs".into())
            .spawn(move || {
                let (stop, wake) = &*stopped;
                let mut done = stop
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                while !*done {
                    let (guard, elapsed) = wake
                        .wait_timeout(done, TICK)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    done = guard;
                    if elapsed.timed_out() {
                        engine.increment_epoch();
                    }
                }
            })?;
        Ok(Self {
            _clock: Arc::new(Clock {
                stop,
                thread: Some(thread),
            }),
            timeout: Some(DEFAULT_EXECUTION_TIMEOUT),
        })
    }

    pub fn begin<T>(&self, store: &mut Store<T>) {
        let ticks = self.timeout.map_or(u64::MAX / 2, |timeout| {
            timeout
                .as_nanos()
                .div_ceil(TICK.as_nanos())
                .min((u64::MAX / 2 - 1) as u128) as u64
                + 1
        });
        store.set_epoch_deadline(ticks);
    }
}
