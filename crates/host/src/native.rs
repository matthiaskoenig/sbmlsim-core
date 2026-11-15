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

use anyhow::Context;
use sbmlsim_abi::{
    self as abi, ModelDescription, ModelSizes, SimulationRequest, SimulationResult,
    SimulationStats, check_contract,
};
use sbmlsim_engine::{EngineError, ModelFns};
use wasmtime::{Memory, MemoryType, Module, Store, TypedFunc};

use crate::{Runtime, check_model_sections, linker_without_wasi};

/// A model loaded into the native engine ([`Runtime::instantiate_native`]).
pub struct NativeInstance {
    engine: sbmlsim_engine::Engine,
}

impl NativeInstance {
    /// Simulates the model; the result does not depend on the simulations before.
    pub fn simulate(&mut self, req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
        self.engine.simulate(req)
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
        check_contract(&desc.contract, abi::MODEL_CONTRACT)?;
        check_model_sections(model_wasm)?;
        let module = Module::new(&self.engine, model_wasm)
            .map_err(anyhow::Error::from)
            .context("model.wasm does not compile")?;
        let mut store = Store::new(&self.engine, ());
        let layout = Layout::of(desc);
        let pages = layout.end.div_ceil(PAGE) as u64;
        let memory = Memory::new(&mut store, MemoryType::new(pages as u32, None))?;
        let mut linker = linker_without_wasi(&self.engine);
        linker.define(&store, "env", "memory", memory)?;
        let math = linker.instantiate(&mut store, &self.mathlib_module)?;
        linker.instance(&mut store, "math", math)?;
        let instance = linker
            .instantiate(&mut store, &module)
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
        let model = WasmModel {
            inner: RefCell::new(Inner { store, memory }),
            functions,
            layout,
            events: desc.events.len(),
        };
        let engine = sbmlsim_engine::Engine::new(Box::new(model), desc.clone())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(NativeInstance { engine })
    }
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
        let np = (sizes.parameters + sizes.constants) as usize;
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
        let bytes = (sizes.parameters + sizes.states).max(sizes.selections) as usize;
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
    store: Store<()>,
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

/// A model of `model/2` in wasmtime behind [`ModelFns`]. A trap of the model is a panic of the
/// native engine (as a trap of `engine.wasm` ends its instance).
struct WasmModel {
    inner: RefCell<Inner>,
    functions: Functions,
    layout: Layout,
    events: usize,
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
fn ok<T>(result: wasmtime::Result<T>) -> T {
    result.unwrap_or_else(|e| panic!("the model trapped: {e:#}"))
}

impl ModelFns for WasmModel {
    fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        ok(self
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
        ok(self.functions.init.call(&mut m.store, args));
        m.get(l.p, p);
        m.get(l.y, y);
    }
    fn rhs(&self, t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        let args = (t, ptr(l.y), ptr(l.p), ptr(l.out));
        ok(self.functions.rhs.call(&mut m.store, args));
        m.get(l.out, ydot);
    }
    fn jac(&self, t: f64, y: &[f64], p: &[f64], values: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        let args = (t, ptr(l.y), ptr(l.p), ptr(l.out));
        ok(self.functions.jac.call(&mut m.store, args));
        m.get(l.out, values);
    }
    fn roots(&self, t: f64, y: &[f64], p: &[f64], g: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        let args = (t, ptr(l.y), ptr(l.p), ptr(l.out));
        ok(self.functions.roots.call(&mut m.store, args));
        m.get(l.out, g);
    }
    fn trigger(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> bool {
        let (mut m, l) = self.enter(y, p);
        let args = (i as i32, t, ptr(l.y), ptr(l.p));
        ok(self.functions.trigger.call(&mut m.store, args)) != 0
    }
    fn priority(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        let (mut m, l) = self.enter(y, p);
        let args = (i as i32, t, ptr(l.y), ptr(l.p));
        ok(self.functions.priority.call(&mut m.store, args))
    }
    fn delay(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        let (mut m, l) = self.enter(y, p);
        let args = (i as i32, t, ptr(l.y), ptr(l.p));
        ok(self.functions.delay.call(&mut m.store, args))
    }
    fn event_values(&self, i: u32, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        let args = (i as i32, t, ptr(l.y), ptr(l.p), ptr(l.out));
        ok(self.functions.event_values.call(&mut m.store, args));
        m.get(l.out, out);
    }
    fn event_apply(&self, i: u32, t: f64, values: &[f64], y: &mut [f64], p: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        m.put(l.values, values);
        let args = (i as i32, t, ptr(l.values), ptr(l.y), ptr(l.p));
        ok(self.functions.event_apply.call(&mut m.store, args));
        m.get(l.y, y);
        m.get(l.p, p);
    }
    fn selections(&self, t: f64, y: &[f64], p: &[f64], mask: &[u8], out: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        m.put_bytes(l.bytes, mask);
        let args = (t, ptr(l.y), ptr(l.p), ptr(l.bytes), ptr(l.out));
        ok(self.functions.selections.call(&mut m.store, args));
        m.get(l.out, out);
    }
    fn assign(&self, t: f64, y: &mut [f64], p: &mut [f64]) {
        let (mut m, l) = self.enter(y, p);
        ok(self
            .functions
            .assign
            .call(&mut m.store, (t, ptr(l.y), ptr(l.p))));
        m.get(l.y, y);
        m.get(l.p, p);
    }
    fn sizes(&self) -> ModelSizes {
        let mut m = self.inner.borrow_mut();
        let l = self.layout;
        ok(self.functions.sizes.call(&mut m.store, ptr(l.out)));
        let mut words = [0u32; ModelSizes::WORDS];
        m.get_words(l.out, &mut words);
        // the counts of assignments only for the events of the description, whose room the
        // layout has
        let events = words[ModelSizes::WORDS - 1] as usize;
        let assignments = if events == self.events {
            ok(self.functions.event_sizes.call(&mut m.store, ptr(l.out)));
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
