//! [`CModel`], the functions of a compiled C model as the [`ModelFns`] of the engine.

use std::os::raw::c_int;
use std::sync::Arc;

use sbmlsim_abi::{ModelDescription, ModelSizes};
use sbmlsim_engine::ModelFns;

use crate::emit::symbol_prefix;
use crate::{CError, Library};

type Defaults = unsafe extern "C" fn(*mut f64, *mut f64);
type Init = unsafe extern "C" fn(f64, *mut f64, *mut f64, *const u8);
type Vectors = unsafe extern "C" fn(f64, *const f64, *const f64, *mut f64);
type Trigger = unsafe extern "C" fn(u32, f64, *const f64, *const f64) -> c_int;
type Scalar = unsafe extern "C" fn(u32, f64, *const f64, *const f64) -> f64;
type EventValues = unsafe extern "C" fn(u32, f64, *const f64, *const f64, *mut f64);
type EventApply = unsafe extern "C" fn(u32, f64, *const f64, *mut f64, *mut f64);
type Selections = unsafe extern "C" fn(f64, *const f64, *const f64, *const u8, *mut f64);
type Assign = unsafe extern "C" fn(f64, *mut f64, *mut f64);
type Sizes = unsafe extern "C" fn(*mut u32);

/// The functions of a library.
#[derive(Clone, Copy)]
struct Functions {
    defaults: Defaults,
    init: Init,
    rhs: Vectors,
    jac: Vectors,
    roots: Vectors,
    trigger: Trigger,
    priority: Scalar,
    delay: Scalar,
    event_values: EventValues,
    event_apply: EventApply,
    selections: Selections,
    assign: Assign,
}

/// The lengths of the vectors the functions read and write.
struct Lengths {
    states: usize,
    parameters: usize,
    /// The parameters and the constants behind them.
    memory: usize,
    selections: usize,
    entries: usize,
    roots: usize,
    assignments: Vec<usize>,
}

struct Inner {
    functions: Functions,
    lengths: Lengths,
    sizes: ModelSizes,
    /// Holds the library of the functions.
    _library: Arc<Library>,
}

/// The model of a [`Library`] of the C of [`crate::generate`]: its functions as the
/// [`ModelFns`] of the engine, one clone per engine (a clone is a reference count).
///
/// Every call checks the lengths of its vectors against the description, so the C never reads
/// or writes outside them.
#[derive(Clone)]
pub struct CModel {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for CModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CModel")
            .field("library", &self.inner._library.path())
            .field("sizes", &self.inner.sizes)
            .finish()
    }
}

impl CModel {
    /// The model of the library of the C of the model of `description`: refuses a library
    /// without the functions of the model ([`CError::Load`]) and one whose sizes are not those
    /// of the description ([`CError::Model`]).
    pub fn new(library: Arc<Library>, description: &ModelDescription) -> Result<CModel, CError> {
        let prefix = symbol_prefix(&description.id);
        let name = |function: &str| format!("{prefix}{function}");
        // SAFETY: the types are the signatures the C of `emit.rs` defines for these names, and
        // the functions are only called while `Inner` holds the library.
        let functions = unsafe {
            Functions {
                defaults: library.symbol(&name("defaults"))?,
                init: library.symbol(&name("init"))?,
                rhs: library.symbol(&name("rhs"))?,
                jac: library.symbol(&name("jac"))?,
                roots: library.symbol(&name("roots"))?,
                trigger: library.symbol(&name("trigger"))?,
                priority: library.symbol(&name("priority"))?,
                delay: library.symbol(&name("delay"))?,
                event_values: library.symbol(&name("event_values"))?,
                event_apply: library.symbol(&name("event_apply"))?,
                selections: library.symbol(&name("selections"))?,
                assign: library.symbol(&name("assign"))?,
            }
        };
        let (words, event_sizes): (Sizes, Sizes) = unsafe {
            (
                library.symbol(&name("sizes"))?,
                library.symbol(&name("event_sizes"))?,
            )
        };
        let mut w = [0u32; ModelSizes::WORDS];
        // SAFETY: `sizes` writes `ModelSizes::WORDS` words, `event_sizes` one per event (the
        // last word).
        unsafe { words(w.as_mut_ptr()) };
        let mut assignments = vec![0u32; w[ModelSizes::WORDS - 1] as usize];
        unsafe { event_sizes(assignments.as_mut_ptr()) };
        let sizes = ModelSizes::from_words(w, assignments);
        let expected = ModelSizes::of(description);
        if sizes != expected {
            return Err(CError::Model(format!(
                "the library {} is not the one of the model {}: its sizes are {sizes:?}, those \
                 of the description {expected:?}",
                library.path().display(),
                description.id
            )));
        }
        let lengths = Lengths {
            states: sizes.states as usize,
            parameters: sizes.parameters as usize,
            memory: (sizes.parameters + sizes.constants) as usize,
            selections: sizes.selections as usize,
            entries: sizes.jacobian_entries as usize,
            roots: sizes.roots as usize,
            assignments: sizes.assignments.iter().map(|&n| n as usize).collect(),
        };
        Ok(CModel {
            inner: Arc::new(Inner {
                functions,
                lengths,
                sizes,
                _library: library,
            }),
        })
    }

    fn functions(&self) -> &Functions {
        &self.inner.functions
    }

    fn lengths(&self) -> &Lengths {
        &self.inner.lengths
    }

    /// Checks `y` and `p` (with the constants) of a function which reads them.
    fn check(&self, y: &[f64], p: &[f64]) {
        let n = self.lengths();
        assert!(
            y.len() >= n.states && p.len() >= n.memory,
            "{}",
            self.shape(y, p)
        );
    }

    fn shape(&self, y: &[f64], p: &[f64]) -> String {
        let n = self.lengths();
        format!(
            "{} states and {} parameters for a model of {} and {} (with the constants)",
            y.len(),
            p.len(),
            n.states,
            n.memory
        )
    }

    /// The number of values of event `i`, 0 for an index which is no event.
    fn assignments(&self, i: u32) -> usize {
        self.lengths()
            .assignments
            .get(i as usize)
            .copied()
            .unwrap_or(0)
    }
}

// SAFETY of the calls below: every function of the library reads and writes the vectors of
// its arguments within the lengths of the sizes of the description, which each call checks
// first; the vectors of one call do not overlap (Rust references), as the `restrict` of the C
// requires; the library is held by `Inner`.
impl ModelFns for CModel {
    fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
        let n = self.lengths();
        assert!(
            y.len() >= n.states && p.len() >= n.parameters,
            "{}",
            self.shape(y, p)
        );
        unsafe { (self.functions().defaults)(p.as_mut_ptr(), y.as_mut_ptr()) }
    }

    fn init(&self, t: f64, p: &mut [f64], y: &mut [f64], fixed: &[u8]) {
        self.check(y, p);
        let n = self.lengths();
        assert!(fixed.len() >= n.parameters + n.states);
        unsafe { (self.functions().init)(t, p.as_mut_ptr(), y.as_mut_ptr(), fixed.as_ptr()) }
    }

    fn rhs(&self, t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
        self.check(y, p);
        assert!(ydot.len() >= self.lengths().states);
        unsafe { (self.functions().rhs)(t, y.as_ptr(), p.as_ptr(), ydot.as_mut_ptr()) }
    }

    fn jac(&self, t: f64, y: &[f64], p: &[f64], values: &mut [f64]) {
        self.check(y, p);
        assert!(values.len() >= self.lengths().entries);
        unsafe { (self.functions().jac)(t, y.as_ptr(), p.as_ptr(), values.as_mut_ptr()) }
    }

    fn roots(&self, t: f64, y: &[f64], p: &[f64], g: &mut [f64]) {
        self.check(y, p);
        assert!(g.len() >= self.lengths().roots);
        unsafe { (self.functions().roots)(t, y.as_ptr(), p.as_ptr(), g.as_mut_ptr()) }
    }

    fn trigger(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> bool {
        self.check(y, p);
        unsafe { (self.functions().trigger)(i, t, y.as_ptr(), p.as_ptr()) != 0 }
    }

    fn priority(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        self.check(y, p);
        unsafe { (self.functions().priority)(i, t, y.as_ptr(), p.as_ptr()) }
    }

    fn delay(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        self.check(y, p);
        unsafe { (self.functions().delay)(i, t, y.as_ptr(), p.as_ptr()) }
    }

    fn event_values(&self, i: u32, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        self.check(y, p);
        assert!(out.len() >= self.assignments(i));
        unsafe { (self.functions().event_values)(i, t, y.as_ptr(), p.as_ptr(), out.as_mut_ptr()) }
    }

    fn event_apply(&self, i: u32, t: f64, values: &[f64], y: &mut [f64], p: &mut [f64]) {
        self.check(y, p);
        assert!(values.len() >= self.assignments(i));
        unsafe {
            (self.functions().event_apply)(i, t, values.as_ptr(), y.as_mut_ptr(), p.as_mut_ptr())
        }
    }

    fn selections(&self, t: f64, y: &[f64], p: &[f64], mask: &[u8], out: &mut [f64]) {
        self.check(y, p);
        let n = self.lengths().selections;
        assert!(mask.len() >= n && out.len() >= n);
        unsafe {
            (self.functions().selections)(
                t,
                y.as_ptr(),
                p.as_ptr(),
                mask.as_ptr(),
                out.as_mut_ptr(),
            )
        }
    }

    fn assign(&self, t: f64, y: &mut [f64], p: &mut [f64]) {
        self.check(y, p);
        unsafe { (self.functions().assign)(t, y.as_mut_ptr(), p.as_mut_ptr()) }
    }

    fn sizes(&self) -> ModelSizes {
        self.inner.sizes.clone()
    }
}
