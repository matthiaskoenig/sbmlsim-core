//! The native engine: `sbmlsim-engine` compiled for the machine of the host (with the CVODE and
//! KLU of the C compiler of the host) instead of `engine.wasm`, which runs a compiled model whose
//! `model.wasm` (and `mathlib.wasm`) wasmtime compiles to native code.
//!
//! It is the configuration `native` of the benchmark: the integrator as fast as Rust and C make
//! it on the machine, with the same model code as the other hosts. Every call of a function of
//! the model copies the vectors the engine lends into the memory of the model and the results
//! back (the model reads and writes nothing but its memory), a few hundred bytes of a model of
//! the size of a PBPK model.
//!
//! Its results are not bitwise those of `engine.wasm`: the C solvers call the `pow` and `sqrt`
//! of the C library of the host (`SUNRpowerR` in the step size of CVODE), `engine.wasm` those of
//! wasi-libc, so the step sizes and with them the results differ in the last bits (within the
//! tolerances of the integration). The math of the model is `mathlib.wasm` in both.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Weak};

use anyhow::Context;
use sbmlsim_abi::{
    self as abi, ModelDescription, ModelSizes, SimulationRequest, SimulationResult, SimulationStats,
};
use sbmlsim_engine::{EngineError, ModelFns};
use wasmtime::{Memory, MemoryType, Store, TypedFunc};

use crate::{MemoryLimit, Runtime, linker_without_wasi};

/// A model loaded into the native engine ([`Runtime::instantiate_native`],
/// [`crate::LoadedModel::instantiate_native`]).
pub struct NativeInstance {
    engine: sbmlsim_engine::Engine,
    inner: Rc<RefCell<Inner>>,
    execution: crate::Execution,
    /// The loaded model this instance is of, which a prepared request is checked against.
    identity: Arc<()>,
    /// The plan of every prepared request this instance resolved, see
    /// [`NativeInstance::simulate_prepared`].
    prepared: BTreeMap<usize, CachedPrepared>,
}

/// The plan of a prepared request in one native engine, with a weak handle to the request it
/// belongs to, so that the plan of a request which was dropped is dropped with it.
struct CachedPrepared {
    owner: Weak<crate::PreparedInner>,
    plan: sbmlsim_engine::PreparedRequest,
}

// SAFETY: an instance owns its whole graph. The two handles of the `Rc` are the field `inner`
// and the one of the `WasmModel` which `engine` owns as its `ModelFns`; `instantiate_native`
// creates both and hands out neither, and nothing clones the `Rc` afterwards, so every handle
// of the count moves with the instance and no two threads ever touch the count at once. What
// the `Rc` holds is a wasmtime `Store`, which is `Send`, and the engine is the C memory of
// CVODE and KLU, which one thread at a time may use and which a move does not touch. `Sync` is
// not implied: an instance simulates one request after the other, and a caller which shares one
// between threads puts it behind a lock, as `Instance` of the python bindings does.
unsafe impl Send for NativeInstance {}

impl NativeInstance {
    /// Simulates the model; the result does not depend on the simulations before.
    pub fn simulate(&mut self, req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
        self.execution.begin(&mut self.inner.borrow_mut().store);
        self.engine.simulate(req)
    }

    /// Simulates a prepared request with one finite value per varying target, in their order.
    ///
    /// The engine resolves the request into a plan of its own the first time this instance
    /// runs it and keeps it for the runs after it, as an instance of `engine.wasm` keeps the
    /// plan of its guest, so a point of a batch transfers only its numbers.
    pub fn simulate_prepared(
        &mut self,
        prepared: &crate::PreparedSimulation,
        values: &[f64],
    ) -> Result<SimulationResult, EngineError> {
        let plan = self.plan_of(prepared)?;
        self.execution.begin(&mut self.inner.borrow_mut().store);
        self.engine.simulate_prepared(&plan, values)
    }

    /// The plan of a prepared request in this engine, resolved once per instance.
    fn plan_of(
        &mut self,
        prepared: &crate::PreparedSimulation,
    ) -> Result<sbmlsim_engine::PreparedRequest, EngineError> {
        if !Arc::ptr_eq(&self.identity, prepared.model()) {
            return Err(EngineError::Request(
                "prepared request belongs to a different loaded model".into(),
            ));
        }
        let (key, owner) = prepared.cache_entry();
        // the plans of the requests which were dropped since the last one go with them
        self.prepared
            .retain(|_, cached| cached.owner.strong_count() > 0);
        if let Some(cached) = self.prepared.get(&key) {
            return Ok(cached.plan.clone());
        }
        let plan = self
            .engine
            .prepare(prepared.request(), prepared.varying())?;
        self.prepared.insert(
            key,
            CachedPrepared {
                owner,
                plan: plan.clone(),
            },
        );
        Ok(plan)
    }

    /// The counters of the integrator of the last simulation.
    pub fn stats(&self) -> SimulationStats {
        self.engine.stats()
    }
}

impl Runtime {
    /// Loads a model into the native engine: its `model.wasm` and `mathlib.wasm` in a store of
    /// their own (without the call hook and the limiter of the instances of `engine.wasm`), the
    /// functions of the model behind [`ModelFns`] for `sbmlsim_engine::Engine`.
    pub fn instantiate_native(
        &self,
        model_wasm: &[u8],
        desc: &ModelDescription,
    ) -> anyhow::Result<NativeInstance> {
        let loaded = self.load(model_wasm, desc)?;
        loaded.instantiate_native()
    }
}

impl crate::LoadedModel {
    /// Loads the model into the native engine, which reuses the `model.wasm` this model
    /// compiled, see [`Runtime::instantiate_native`]. Every instance is independent, and the
    /// result of a simulation does not depend on the ones before it.
    pub fn instantiate_native(&self) -> anyhow::Result<NativeInstance> {
        native_instance(
            self.wasmtime_engine(),
            self.mathlib(),
            self.model_module(),
            self.description(),
            self.max_memory(),
            self.execution(),
            self.identity(),
        )
    }
}

/// Instantiates `model_wasm` into the native engine, see [`Runtime::instantiate_native`].
fn native_instance(
    wasmtime_engine: &wasmtime::Engine,
    mathlib_module: &wasmtime::Module,
    module: &wasmtime::Module,
    desc: &ModelDescription,
    max_memory: usize,
    execution: &crate::Execution,
    identity: &Arc<()>,
) -> anyhow::Result<NativeInstance> {
    let mut store = Store::new(
        wasmtime_engine,
        MemoryLimit {
            max_bytes: max_memory,
            grows: 0,
        },
    );
    store.limiter(|limit| limit);
    execution.begin(&mut store);
    let layout = Layout::of(desc);
    anyhow::ensure!(
        layout.end <= max_memory && layout.end <= i32::MAX as usize,
        "the model vectors do not fit into the runtime memory limit"
    );
    let pages = layout.end.div_ceil(PAGE) as u64;
    let memory = Memory::new(&mut store, MemoryType::new(pages as u32, None))?;
    let mut linker = linker_without_wasi(wasmtime_engine);
    linker.define(&store, "env", "memory", memory)?;
    let math = linker.instantiate(&mut store, mathlib_module)?;
    linker.instance(&mut store, "math", math)?;
    let instance = linker
        .instantiate(&mut store, module)
        .map_err(anyhow::Error::from)
        .context("model.wasm does not instantiate")?;
    let mut get = |name: &str| {
        instance
            .get_func(&mut store, name)
            .with_context(|| format!("model.wasm does not export {name}"))
    };
    let (defaults, init, rhs, jac, roots) = (
        get(abi::DEFAULTS)?,
        get(abi::INIT)?,
        get(abi::RHS)?,
        get(abi::JAC)?,
        get(abi::ROOTS)?,
    );
    let (trigger, priority, delay, event_values, event_apply) = (
        get(abi::TRIGGER)?,
        get(abi::PRIORITY)?,
        get(abi::DELAY)?,
        get(abi::EVENT_VALUES)?,
        get(abi::EVENT_APPLY)?,
    );
    let (selections, assign, sizes, event_sizes) = (
        get(abi::SELECTIONS)?,
        get(abi::ASSIGN)?,
        get(abi::SIZES)?,
        get(abi::EVENT_SIZES)?,
    );
    let functions = Functions {
        defaults: defaults.typed(&store)?,
        init: init.typed(&store)?,
        rhs: rhs.typed(&store)?,
        jac: jac.typed(&store)?,
        roots: roots.typed(&store)?,
        trigger: trigger.typed(&store)?,
        priority: priority.typed(&store)?,
        delay: delay.typed(&store)?,
        event_values: event_values.typed(&store)?,
        event_apply: event_apply.typed(&store)?,
        selections: selections.typed(&store)?,
        assign: assign.typed(&store)?,
        sizes: sizes.typed(&store)?,
        event_sizes: event_sizes.typed(&store)?,
    };
    let inner = Rc::new(RefCell::new(Inner { store, memory }));
    let model = WasmModel {
        inner: inner.clone(),
        functions,
        layout,
        events: desc.events.len(),
        error: RefCell::new(None),
    };
    // an instance of `engine.wasm` is bounded by the memory of its guest, which bounds its
    // result with it; the native engine allocates its result in the memory of the host, so the
    // limit of the instance is given to it as the values a result may hold
    let engine = sbmlsim_engine::Engine::new(Box::new(model), desc.clone())
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .with_max_output_values(max_memory / size_of::<f64>());
    Ok(NativeInstance {
        engine,
        inner,
        identity: identity.clone(),
        prepared: BTreeMap::new(),
        execution: execution.clone(),
    })
}

/// The bytes of a page of wasm.
const PAGE: usize = 65536;

/// The typed exports of a model.
struct Functions {
    defaults: TypedFunc<(i32, i32), ()>,
    init: TypedFunc<(f64, i32, i32, i32), ()>,
    rhs: TypedFunc<(f64, i32, i32, i32), ()>,
    jac: TypedFunc<(f64, i32, i32, i32), ()>,
    roots: TypedFunc<(f64, i32, i32, i32), ()>,
    trigger: TypedFunc<(i32, f64, i32, i32), i32>,
    priority: TypedFunc<(i32, f64, i32, i32), f64>,
    delay: TypedFunc<(i32, f64, i32, i32), f64>,
    event_values: TypedFunc<(i32, f64, i32, i32, i32), ()>,
    event_apply: TypedFunc<(i32, f64, i32, i32, i32), ()>,
    selections: TypedFunc<(f64, i32, i32, i32, i32), ()>,
    assign: TypedFunc<(f64, i32, i32), ()>,
    sizes: TypedFunc<i32, ()>,
    event_sizes: TypedFunc<i32, ()>,
}

/// Where the vectors the engine lends live in the memory of the model: every region aligned to
/// 8 and as long as the description says the vector is.
#[derive(Debug, Clone, Copy)]
struct Layout {
    y: usize,
    p: usize,
    out: usize,
    values: usize,
    bytes: usize,
    end: usize,
}

impl Layout {
    fn of(desc: &ModelDescription) -> Layout {
        let sizes = ModelSizes::of(desc);
        let n = sizes.states as usize;
        let np = sizes.parameters as usize + sizes.constants as usize;
        let assignments = sizes.assignments.iter().copied().max().unwrap_or(0) as usize;
        // the outputs: ydot, the Jacobian, the root functions, the selections, the event values
        // and the words of the sizes (as many as the events, at least the words)
        let out = [
            n,
            sizes.jacobian_entries as usize,
            sizes.roots as usize,
            sizes.selections as usize,
            assignments,
            ModelSizes::WORDS.max(sizes.events as usize),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        // bytes: `fixed` (a byte per entity) or the mask (a byte per selection)
        let bytes =
            (sizes.parameters as usize + sizes.states as usize).max(sizes.selections as usize);
        let y = 0;
        let p = y + 8 * n;
        let out_at = p + 8 * np;
        let values = out_at + 8 * out;
        let bytes_at = values + 8 * assignments;
        Layout {
            y,
            p,
            out: out_at,
            values,
            bytes: bytes_at,
            end: (bytes_at + bytes).next_multiple_of(8).max(8),
        }
    }
}

/// The store of the model and its memory.
struct Inner {
    store: Store<MemoryLimit>,
    memory: Memory,
}

impl Inner {
    fn put(&mut self, at: usize, v: &[f64]) {
        let data = &mut self.memory.data_mut(&mut self.store)[at..at + 8 * v.len()];
        for (chunk, x) in data.as_chunks_mut::<8>().0.iter_mut().zip(v) {
            *chunk = x.to_le_bytes();
        }
    }

    fn get(&mut self, at: usize, v: &mut [f64]) {
        let data = &self.memory.data(&self.store)[at..at + 8 * v.len()];
        for (chunk, x) in data.as_chunks::<8>().0.iter().zip(v) {
            *x = f64::from_le_bytes(*chunk);
        }
    }

    fn put_bytes(&mut self, at: usize, v: &[u8]) {
        self.memory.data_mut(&mut self.store)[at..at + v.len()].copy_from_slice(v);
    }

    fn get_words(&mut self, at: usize, v: &mut [u32]) {
        let data = &self.memory.data(&self.store)[at..at + 4 * v.len()];
        for (chunk, x) in data.as_chunks::<4>().0.iter().zip(v) {
            *x = u32::from_le_bytes(*chunk);
        }
    }
}

/// A model of `model/2` in wasmtime behind [`ModelFns`]. The first trap is recorded and
/// returned by the engine as an internal error, without unwinding across the C ABI.
struct WasmModel {
    inner: Rc<RefCell<Inner>>,
    functions: Functions,
    layout: Layout,
    events: usize,
    error: RefCell<Option<String>>,
}

/// An offset as the `i32` of a pointer of wasm.
fn ptr(at: usize) -> i32 {
    i32::try_from(at).expect("the memory of a model is below 2 GiB")
}

impl WasmModel {
    /// The store, after `y` and `p` are written into the memory, and the layout.
    fn enter(&self, y: &[f64], p: &[f64]) -> (std::cell::RefMut<'_, Inner>, Layout) {
        let mut inner = self.inner.borrow_mut();
        let l = self.layout;
        inner.put(l.y, y);
        inner.put(l.p, p);
        (inner, l)
    }
}

/// The result of a call of the model, which traps only for a defect of the model.
impl WasmModel {
    fn ok<T: Default>(&self, result: wasmtime::Result<T>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => {
                self.error
                    .borrow_mut()
                    .get_or_insert_with(|| format!("the model trapped: {error:#}"));
                T::default()
            }
        }
    }
}

impl ModelFns for WasmModel {
    fn check_error(&self) -> Result<(), EngineError> {
        match &*self.error.borrow() {
            Some(error) => Err(EngineError::Internal(error.clone())),
            None => Ok(()),
        }
    }

    fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        self.ok(self
            .functions
            .defaults
            .call(&mut m.store, (ptr(l.p), ptr(l.y))));
        m.get(l.p, p);
        m.get(l.y, y);
    }
    fn init(&self, t: f64, p: &mut [f64], y: &mut [f64], fixed: &[u8]) {
        let (mut m, l) = self.enter(y, p);
        m.put_bytes(l.bytes, fixed);
        let args = (t, ptr(l.p), ptr(l.y), ptr(l.bytes));
        self.ok(self.functions.init.call(&mut m.store, args));
        m.get(l.p, p);
        m.get(l.y, y);
    }
    fn rhs(&self, t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        let args = (t, ptr(l.y), ptr(l.p), ptr(l.out));
        self.ok(self.functions.rhs.call(&mut m.store, args));
        m.get(l.out, ydot);
    }
    fn jac(&self, t: f64, y: &[f64], p: &[f64], values: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        let args = (t, ptr(l.y), ptr(l.p), ptr(l.out));
        self.ok(self.functions.jac.call(&mut m.store, args));
        m.get(l.out, values);
    }
    fn roots(&self, t: f64, y: &[f64], p: &[f64], g: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        let args = (t, ptr(l.y), ptr(l.p), ptr(l.out));
        self.ok(self.functions.roots.call(&mut m.store, args));
        m.get(l.out, g);
    }
    fn trigger(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> bool {
        let (mut m, l) = self.enter(y, p);
        let args = (i as i32, t, ptr(l.y), ptr(l.p));
        self.ok(self.functions.trigger.call(&mut m.store, args)) != 0
    }
    fn priority(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        let (mut m, l) = self.enter(y, p);
        let args = (i as i32, t, ptr(l.y), ptr(l.p));
        self.ok(self.functions.priority.call(&mut m.store, args))
    }
    fn delay(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        let (mut m, l) = self.enter(y, p);
        let args = (i as i32, t, ptr(l.y), ptr(l.p));
        self.ok(self.functions.delay.call(&mut m.store, args))
    }
    fn event_values(&self, i: u32, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        let args = (i as i32, t, ptr(l.y), ptr(l.p), ptr(l.out));
        self.ok(self.functions.event_values.call(&mut m.store, args));
        m.get(l.out, out);
    }
    fn event_apply(&self, i: u32, t: f64, values: &[f64], y: &mut [f64], p: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        m.put(l.values, values);
        let args = (i as i32, t, ptr(l.values), ptr(l.y), ptr(l.p));
        self.ok(self.functions.event_apply.call(&mut m.store, args));
        m.get(l.y, y);
        m.get(l.p, p);
    }
    fn selections(&self, t: f64, y: &[f64], p: &[f64], mask: &[u8], out: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        m.put_bytes(l.bytes, mask);
        let args = (t, ptr(l.y), ptr(l.p), ptr(l.bytes), ptr(l.out));
        self.ok(self.functions.selections.call(&mut m.store, args));
        m.get(l.out, out);
    }
    fn assign(&self, t: f64, y: &mut [f64], p: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        self.ok(self
            .functions
            .assign
            .call(&mut m.store, (t, ptr(l.y), ptr(l.p))));
        m.get(l.y, y);
        m.get(l.p, p);
    }
    fn sizes(&self) -> ModelSizes {
        let mut m = self.inner.borrow_mut();
        let l = self.layout;
        self.ok(self.functions.sizes.call(&mut m.store, ptr(l.out)));
        let mut words = [0u32; ModelSizes::WORDS];
        m.get_words(l.out, &mut words);
        // the counts of assignments only for the events of the description, whose room the
        // layout has
        let events = words[ModelSizes::WORDS - 1] as usize;
        let assignments = if events == self.events {
            self.ok(self.functions.event_sizes.call(&mut m.store, ptr(l.out)));
            let mut assignments = vec![0u32; events];
            m.get_words(l.out, &mut assignments);
            assignments
        } else {
            Vec::new()
        };
        ModelSizes::from_words(words, assignments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_keeps_the_vectors_apart() {
        let desc = sbmlsim_engine::model::Decay { k: 0.5 }.description();
        let l = Layout::of(&desc);
        assert!(l.y < l.p && l.p < l.out && l.out <= l.values && l.values <= l.bytes);
        assert!(l.end.is_multiple_of(8) && l.end > l.bytes);
    }
}
