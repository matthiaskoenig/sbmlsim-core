//! Consistent initialization when initial assignments depend on algebraic unknowns.
//!
//! For a candidate algebraic suffix `a`, `model_init` gives the other initial values
//! `H(a)`. Solving `g(H(a), a) = 0` together accounts for initial assignments and
//! concentration conversions. Alternating initialization and projection need not converge.

use std::cell::RefCell;

use sbmlsim_abi::{
    IntegratorSettings, JacobianChoice, JacobianKind, LinearSolver, ModelDescription, ModelSizes,
    SparsityPattern, Tolerance,
};

use crate::changes::Target;
use crate::solver::{ConstraintWorkspace, Constraints, Counts, SolverCounts};
use crate::{EngineError, INIT_TIME, ModelFns};

struct Workspace {
    p: Vec<f64>,
    y: Vec<f64>,
    rates: Vec<f64>,
    fixed: Vec<u8>,
}

struct Initialization<'a> {
    model: &'a dyn ModelFns,
    differential: usize,
    y: &'a [f64],
    preinit: &'a [(Target, f64)],
    fixed: &'a [u8],
    n_parameters: usize,
    sizes: ModelSizes,
    workspace: RefCell<&'a mut Workspace>,
}

impl Initialization<'_> {
    fn evaluate(&self, algebraic: &[f64], workspace: &mut Workspace) {
        let Workspace { p, y, fixed, .. } = workspace;
        self.model.defaults(p, y);
        fixed.copy_from_slice(self.fixed);
        let np = self.n_parameters;
        for &(target, value) in self.preinit {
            if target.compartment().is_none() {
                let index = target.index();
                if index < np {
                    p[index] = value;
                } else {
                    y[index - np] = value;
                }
            }
        }
        y[self.differential..].copy_from_slice(algebraic);
        self.model.init(INIT_TIME, p, y, fixed);
        let mut converted = false;
        for &(target, value) in self.preinit {
            if let Some(compartment) = target.compartment() {
                let size = if compartment < np {
                    p[compartment]
                } else {
                    y[compartment - np]
                };
                let stored = target.stored(value, size);
                let index = target.index();
                if index < np {
                    p[index] = stored;
                } else {
                    y[index - np] = stored;
                }
                fixed[index] = 1;
                converted = true;
            }
        }
        if converted {
            self.model.init(INIT_TIME, p, y, fixed);
        }
    }
}

impl ModelFns for Initialization<'_> {
    fn check_error(&self) -> Result<(), EngineError> {
        self.model.check_error()
    }

    fn rhs(&self, _t: f64, algebraic: &[f64], _p: &[f64], out: &mut [f64]) {
        let mut workspace = self.workspace.borrow_mut();
        self.evaluate(algebraic, &mut workspace);
        let Workspace { p, y, rates, .. } = &mut **workspace;
        self.model.rhs(INIT_TIME, y, p, rates);
        out.copy_from_slice(&rates[self.differential..]);
    }

    fn defaults(&self, _p: &mut [f64], y: &mut [f64]) {
        y.copy_from_slice(&self.y[self.differential..]);
    }
    fn init(&self, _t: f64, _p: &mut [f64], _y: &mut [f64], _fixed: &[u8]) {}
    fn jac(&self, _t: f64, _y: &[f64], _p: &[f64], _values: &mut [f64]) {
        unreachable!("initialization uses a difference Jacobian")
    }
    fn roots(&self, _t: f64, _y: &[f64], _p: &[f64], _g: &mut [f64]) {}
    fn trigger(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64]) -> bool {
        false
    }
    fn priority(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64]) -> f64 {
        0.0
    }
    fn delay(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64]) -> f64 {
        0.0
    }
    fn event_values(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64], _out: &mut [f64]) {}
    fn event_apply(&self, _i: u32, _t: f64, _v: &[f64], _y: &mut [f64], _p: &mut [f64]) {}
    fn selections(&self, _t: f64, _y: &[f64], _p: &[f64], _mask: &[u8], _out: &mut [f64]) {}
    fn assign(&self, _t: f64, _y: &mut [f64], _p: &mut [f64]) {}
    fn sizes(&self) -> ModelSizes {
        self.sizes.clone()
    }
}

/// Metadata and storage for the coupled algebraic initialization of one model.
pub(crate) struct Cache {
    description: ModelDescription,
    sizes: ModelSizes,
    fixed: Vec<u8>,
    workspace: Workspace,
    algebraic: Vec<f64>,
    constraints: Option<ConstraintWorkspace>,
    requested: Option<IntegratorSettings>,
    settings: IntegratorSettings,
}

impl Cache {
    fn new(
        desc: &ModelDescription,
        p: &[f64],
        y: &[f64],
        fixed: &[u8],
    ) -> Result<Self, EngineError> {
        let n = desc.n_algebraic as usize;
        let differential = y.len() - n;
        let entries = n
            .checked_mul(n)
            .and_then(|entries| u32::try_from(entries).ok())
            .ok_or_else(|| {
                EngineError::Request("the algebraic initialization is too large".into())
            })?;
        let mut col_ptr = Vec::new();
        let mut row_idx = Vec::new();
        col_ptr.try_reserve_exact(n + 1).map_err(|_| {
            EngineError::Request("the algebraic initialization does not fit into memory".into())
        })?;
        row_idx.try_reserve_exact(entries as usize).map_err(|_| {
            EngineError::Request("the algebraic initialization does not fit into memory".into())
        })?;
        col_ptr.extend((0..=n).map(|i| (i * n) as u32));
        row_idx.extend((0..n).flat_map(|_| 0..n as u32));
        let pattern = SparsityPattern {
            n: n as u32,
            col_ptr,
            row_idx,
        };
        let description = ModelDescription {
            jacobian_p: None,
            states: desc.states[differential..].to_vec(),
            parameters: Vec::new(),
            selections: Vec::new(),
            jacobian: JacobianKind::ColoredDifference {
                pattern,
                colors: (0..n as u32).collect(),
                reason: "coupled algebraic initial conditions".into(),
            },
            events: Vec::new(),
            n_roots: 0,
            n_discontinuity_roots: 0,
            n_constants: 0,
            driven: Vec::new(),
            t_ref: None,
            ..desc.clone()
        };
        Ok(Self {
            sizes: ModelSizes::of(&description),
            description,
            fixed: fixed.to_vec(),
            workspace: Workspace {
                p: p.to_vec(),
                y: y.to_vec(),
                rates: vec![0.0; y.len()],
                fixed: fixed.to_vec(),
            },
            algebraic: y[differential..].to_vec(),
            constraints: None,
            requested: None,
            settings: IntegratorSettings::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn storage_identity(&self) -> [usize; 8] {
        [
            self.description.jacobian.pattern().row_idx.as_ptr() as usize,
            self.workspace.p.as_ptr() as usize,
            self.workspace.y.as_ptr() as usize,
            self.workspace.rates.as_ptr() as usize,
            self.workspace.fixed.as_ptr() as usize,
            self.fixed.as_ptr() as usize,
            self.algebraic.as_ptr() as usize,
            self.constraints.as_ref().unwrap().storage_identity().0,
        ]
    }

    fn configure(&mut self, settings: &IntegratorSettings, differential: usize) {
        if self.requested.as_ref() == Some(settings) {
            return;
        }
        self.settings = IntegratorSettings {
            linear_solver: LinearSolver::Dense,
            jacobian: JacobianChoice::DifferenceQuotient,
            absolute_tolerance: match &settings.absolute_tolerance {
                Tolerance::PerState(values) => Tolerance::PerState(values[differential..].to_vec()),
                tolerance => tolerance.clone(),
            },
            ..settings.clone()
        };
        self.requested = Some(settings.clone());
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn consistent(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    preinit: &[(Target, f64)],
    scratch: &mut crate::Scratch,
    settings: &IntegratorSettings,
) -> Result<(Counts, SolverCounts), EngineError> {
    let n = desc.n_algebraic as usize;
    debug_assert!(n > 0);
    let crate::Scratch {
        fixed, p, y, dae, ..
    } = scratch;
    let differential = y.len() - n;
    let cache = &mut dae.get_or_insert_with(Box::default).initialization;
    if cache.is_none() {
        *cache = Some(Cache::new(desc, p, y, fixed)?);
    }
    let cache = cache.as_mut().expect("initialized algebraic workspace");
    cache.configure(settings, differential);
    cache.fixed.copy_from_slice(fixed);
    for &(target, _) in preinit {
        if target.compartment().is_some() {
            cache.fixed[target.index()] = 0;
        }
    }
    cache.fixed[desc.parameters.len() + differential..].fill(1);
    cache.workspace.p.copy_from_slice(p);
    cache.workspace.y.copy_from_slice(y);
    cache.algebraic.copy_from_slice(&y[differential..]);
    let initialization = Initialization {
        model,
        differential,
        y,
        preinit,
        fixed: &cache.fixed,
        n_parameters: desc.parameters.len(),
        sizes: cache.sizes.clone(),
        workspace: RefCell::new(&mut cache.workspace),
    };
    let mut constraints = Constraints::cached(
        &initialization,
        &cache.description,
        &cache.settings,
        cache.constraints.take(),
    )?;
    let projected = constraints.project(INIT_TIME, &mut cache.algebraic, &[]);
    let counts = constraints.counts();
    cache.constraints = Some(constraints.into_workspace());
    projected?;
    {
        let mut workspace = initialization.workspace.borrow_mut();
        initialization.evaluate(&cache.algebraic, &mut workspace);
    }
    model.check_error()?;
    drop(initialization);
    p.copy_from_slice(&cache.workspace.p);
    y.copy_from_slice(&cache.workspace.y);
    Ok(counts)
}
