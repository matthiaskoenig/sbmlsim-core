//! The PEtab v2 problems of `sbmlsim-petab` as objects of python, `sbmlsim_core.petab`.
//!
//! A problem is read, compiled and evaluated in Rust: the files are read by the reader of the
//! crate (from a directory, or from bytes a caller gives), the math of its tables is the math
//! of the compiler, every simulation runs on the native engine and the GIL is held for none of
//! it.

use std::collections::HashMap;

use numpy::PyArray1;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use sbmlsim_abi::{IntegratorSettings, Tolerance};
use sbmlsim_petab::{
    DataAtChange, Evaluation, Executor, ExtensionPolicy, Options, PetabError, PetabProblem, Plan,
};

use crate::errors::{CompileError, SbmlsimCoreError, UnsupportedError};

fn petab_error(error: PetabError) -> PyErr {
    match error {
        PetabError::Compile { .. } => CompileError::new_err(error.to_string()),
        PetabError::Unsupported(_) => UnsupportedError::new_err(error.to_string()),
        PetabError::Engine(_) => SbmlsimCoreError::new_err(error.to_string()),
        _ => PyValueError::new_err(error.to_string()),
    }
}

/// The options of a plan from the keywords of python.
fn options(
    relative_tolerance: Option<f64>,
    absolute_tolerance: Option<f64>,
    data_at_change: Option<&str>,
    extension: &str,
) -> PyResult<Options> {
    let mut options = Options::default();
    if relative_tolerance.is_some() || absolute_tolerance.is_some() {
        let mut settings = IntegratorSettings {
            relative_tolerance: 1e-8,
            ..IntegratorSettings::default()
        };
        if let Some(relative) = relative_tolerance {
            settings.relative_tolerance = relative;
        }
        if let Some(absolute) = absolute_tolerance {
            settings.absolute_tolerance = Tolerance::Scalar(absolute);
        }
        options.integrator = Some(settings);
    }
    options.data_at_change = match data_at_change {
        None => None,
        Some("after") => Some(DataAtChange::After),
        Some("before") => Some(DataAtChange::Before),
        Some(other) => {
            return Err(PyValueError::new_err(format!(
                "data_at_change is 'after' or 'before', not '{other}'"
            )));
        }
    };
    options.extension = match extension {
        "read" => ExtensionPolicy::Read,
        "ignore" => ExtensionPolicy::Ignore,
        other => {
            return Err(PyValueError::new_err(format!(
                "extension is 'read' or 'ignore', not '{other}'"
            )));
        }
    };
    Ok(options)
}

/// A PEtab v2 problem, compiled and loaded into the native engine.
#[pyclass(frozen, module = "sbmlsim_core", name = "PetabProblem")]
pub(crate) struct PetabProblemPy {
    executor: Executor,
}

impl PetabProblemPy {
    fn plan(&self) -> &Plan {
        self.executor.plan()
    }

    /// The values of the estimated parameters, their nominal values without.
    fn point(&self, x: Option<Vec<f64>>) -> PyResult<Vec<f64>> {
        let wanted = self.plan().estimated().len();
        match x {
            None => Ok(self.plan().nominal()),
            Some(x) if x.len() == wanted => Ok(x),
            Some(x) => Err(PyValueError::new_err(format!(
                "the problem estimates {wanted} parameters and the point has {} values",
                x.len()
            ))),
        }
    }

    fn of(problem: &PetabProblem, options: &Options) -> PyResult<Self> {
        let plan = Plan::new(problem, options).map_err(petab_error)?;
        let executor = Executor::new(plan).map_err(petab_error)?;
        Ok(PetabProblemPy { executor })
    }
}

fn evaluation_dict<'py>(
    py: Python<'py>,
    plan: &Plan,
    evaluation: Evaluation,
) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("llh", evaluation.llh)?;
    out.set_item("chi2", evaluation.chi2)?;
    out.set_item("log_prior", evaluation.log_prior)?;
    out.set_item("unnorm_log_posterior", evaluation.unnorm_log_posterior())?;
    let priors = PyDict::new(py);
    for (id, value) in &evaluation.log_priors {
        priors.set_item(id, value)?;
    }
    out.set_item("log_priors", priors)?;
    out.set_item("simulation_table", plan.simulation_table(&evaluation))?;
    out.set_item("simulated", PyArray1::from_vec(py, evaluation.simulated))?;
    out.set_item("sigma", PyArray1::from_vec(py, evaluation.sigma))?;
    out.set_item("failures", evaluation.failures)?;
    Ok(out)
}

#[pymethods]
impl PetabProblemPy {
    /// Reads a problem from its YAML file, with the files it names next to it.
    ///
    /// `relative_tolerance` and `absolute_tolerance` are the ones of the integrator (the ones
    /// of the extension `sbmlsim` of the problem, else `1e-8` and `1e-12`),
    /// `data_at_change` the state a measurement at a change reads (`"after"`, PEtab's, unless
    /// the extension says otherwise) and `extension` whether the extension `sbmlsim` is read
    /// (`"read"`) or ignored (`"ignore"`).
    #[staticmethod]
    #[pyo3(signature = (yaml, *, relative_tolerance=None, absolute_tolerance=None, data_at_change=None, extension="read"))]
    fn from_yaml(
        py: Python<'_>,
        yaml: std::path::PathBuf,
        relative_tolerance: Option<f64>,
        absolute_tolerance: Option<f64>,
        data_at_change: Option<&str>,
        extension: &str,
    ) -> PyResult<Self> {
        let options = options(
            relative_tolerance,
            absolute_tolerance,
            data_at_change,
            extension,
        )?;
        py.detach(|| {
            let problem = PetabProblem::from_path(yaml).map_err(petab_error)?;
            PetabProblemPy::of(&problem, &options)
        })
    }

    /// Reads a problem from the contents of its files, by their names: the YAML file `yaml`
    /// and every file it names, as `bytes`.
    #[staticmethod]
    #[pyo3(signature = (files, yaml, *, relative_tolerance=None, absolute_tolerance=None, data_at_change=None, extension="read"))]
    fn from_files(
        py: Python<'_>,
        files: HashMap<String, Vec<u8>>,
        yaml: &str,
        relative_tolerance: Option<f64>,
        absolute_tolerance: Option<f64>,
        data_at_change: Option<&str>,
        extension: &str,
    ) -> PyResult<Self> {
        let options = options(
            relative_tolerance,
            absolute_tolerance,
            data_at_change,
            extension,
        )?;
        py.detach(|| {
            let problem = PetabProblem::read(yaml, &files).map_err(petab_error)?;
            PetabProblemPy::of(&problem, &options)
        })
    }

    /// The ids of the estimated parameters, in the order of a point.
    #[getter]
    fn parameters(&self) -> Vec<String> {
        self.plan()
            .estimated()
            .into_iter()
            .map(|k| self.plan().parameters[k].id.clone())
            .collect()
    }

    /// The nominal values of the estimated parameters, where PEtab evaluates a problem.
    #[getter]
    fn nominal(&self) -> Vec<f64> {
        self.plan().nominal()
    }

    /// The bounds of the estimated parameters, as `(lower, upper)`.
    #[getter]
    fn bounds(&self) -> (Vec<f64>, Vec<f64>) {
        self.plan()
            .estimated()
            .into_iter()
            .map(|k| {
                (
                    self.plan().parameters[k].lower,
                    self.plan().parameters[k].upper,
                )
            })
            .unzip()
    }

    /// The number of measurements.
    #[getter]
    fn n_measurements(&self) -> usize {
        self.plan().measurements.len()
    }

    /// The number of simulations of an evaluation, one per experiment of a model.
    #[getter]
    fn n_simulations(&self) -> usize {
        self.plan().simulations.len()
    }

    /// The requests of `simulation/1` of every simulation at a point, the nominal values
    /// without, as dicts.
    #[pyo3(signature = (x=None))]
    fn requests<'py>(&self, py: Python<'py>, x: Option<Vec<f64>>) -> PyResult<Bound<'py, PyAny>> {
        let x = self.point(x)?;
        pythonize::pythonize(py, &self.plan().requests(&x))
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    /// Evaluates the problem at a point, the nominal values without: a dict of `llh`, `chi2`,
    /// `log_prior`, `log_priors` (by parameter), `unnorm_log_posterior`, `simulated` and
    /// `sigma` (one per measurement, in the order of the measurement table),
    /// `simulation_table` (the TSV of PEtab) and `failures` (the simulations which failed,
    /// with their messages).
    #[pyo3(signature = (x=None))]
    fn evaluate<'py>(&self, py: Python<'py>, x: Option<Vec<f64>>) -> PyResult<Bound<'py, PyDict>> {
        let x = self.point(x)?;
        let evaluation = py.detach(|| self.executor.evaluate(&x));
        evaluation_dict(py, self.plan(), evaluation)
    }

    /// The log-likelihood of PEtab at a point, the nominal values without.
    #[pyo3(signature = (x=None))]
    fn llh(&self, py: Python<'_>, x: Option<Vec<f64>>) -> PyResult<f64> {
        let x = self.point(x)?;
        Ok(py.detach(|| self.executor.evaluate(&x)).llh)
    }

    /// The problem of the fit of the engine, a `FitProblem` whose cost is the negative log
    /// posterior of PEtab plus `cost_offset`, for an optimizer which stays in python.
    #[pyo3(signature = (*, threads=None))]
    fn fit_problem(
        &self,
        py: Python<'_>,
        threads: Option<usize>,
    ) -> PyResult<crate::fit::FitProblem> {
        let threads = threads.unwrap_or_else(default_threads);
        let problem = py
            .detach(|| self.executor.problem(threads))
            .map_err(petab_error)?;
        Ok(crate::fit::FitProblem::new(problem))
    }

    /// What the cost of `fit_problem` carries on top of PEtab's objective.
    #[getter]
    fn cost_offset(&self) -> PyResult<f64> {
        self.executor.cost_offset().map_err(petab_error)
    }

    /// Fits the estimated parameters from their nominal values with an optimizer of the
    /// engine; `settings` is the `optimizer` of a request of `fit/1` (`{"algorithm":
    /// "trust_region", "max_iterations": 100}` by default). Gives a dict of the values `x`,
    /// PEtab's `objective` (the negative log posterior, the negative log-likelihood of a
    /// problem without priors), the `status` of the optimizer and its counts.
    #[pyo3(signature = (settings=None, *, threads=None))]
    fn fit<'py>(
        &self,
        py: Python<'py>,
        settings: Option<&Bound<'_, PyAny>>,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let settings: sbmlsim_fit::Settings = match settings {
            None => sbmlsim_fit::Settings::default(),
            Some(settings) => serde_path_to_error::deserialize(
                &mut pythonize::Depythonizer::from_object(settings),
            )
            .map_err(|error| {
                PyValueError::new_err(format!("settings: {}: {}", error.path(), error.inner()))
            })?,
        };
        let threads = threads.unwrap_or_else(default_threads);
        let answer = py
            .detach(|| self.executor.fit(&settings, threads))
            .map_err(petab_error)?;
        let out = PyDict::new(py);
        out.set_item("x", answer.x)?;
        out.set_item("objective", answer.objective)?;
        out.set_item("cost", answer.fit.cost)?;
        out.set_item("status", answer.fit.status)?;
        out.set_item("iterations", answer.fit.iterations)?;
        out.set_item("evaluations", answer.fit.evaluations)?;
        out.set_item("seconds", answer.fit.seconds)?;
        Ok(out)
    }

    fn __repr__(&self) -> String {
        format!(
            "Problem(parameters={}, measurements={}, simulations={}, models={})",
            self.plan().estimated().len(),
            self.plan().measurements.len(),
            self.plan().simulations.len(),
            self.plan().models.len()
        )
    }
}

fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}
