//! The time of the standard library under the name of [web-time](https://crates.io/crates/web-time).
//!
//! basin, the optimizers of `sbmlsim-fit`, reads the clock of the time limit of a run
//! (`Executor::max_time`) from web-time, which is the standard library on every target but
//! `wasm32-unknown-unknown`, where it is the `performance.now()` of JavaScript through
//! wasm-bindgen. A module of the workspace for that target is no module of wasm-bindgen:
//! `compiler.wasm` is loaded by `js/compile.mjs`, by the python package and by the command line
//! without imports, and the bindings of js-sys alone would add 0.8 MB of exports to it and
//! imports none of them provides. The fit of a page (`sbmlsim_fit::Asking`) never reads the
//! clock, the host measures its time, so the workspace replaces web-time with this crate
//! (`[patch.crates-io]`): the standard library on every target, whose clock is the one web-time
//! re-exports natively and which panics in `wasm32-unknown-unknown`, as a read of a clock which
//! is not there should.

pub use std::time::{Duration, Instant, SystemTime, SystemTimeError, UNIX_EPOCH};
