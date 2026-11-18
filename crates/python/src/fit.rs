//! The problem of a fit as an object of python: the objective of the engine under an
//! optimizer of a caller.
//!
//! [`crate::runtime::LoadedModel::fit`] runs the whole fit in the engine, which is what a
//! host wants once its problem is a request of `fit/1`. A host whose optimizer stays in
//! python (scipy, a sampler, a fit of its own reports) asks for the residuals and the
//! jacobian of a point instead: every call is one batch of the engine, the residuals of a
//! point are computed on the thread of the point, and the GIL is held for none of it.

use numpy::{PyArray1, PyArray2};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::errors::fit_error;

/// The problem of a fit of the contract `fit/1`, resolved against a model and the pool of
/// its batch: the residuals, the cost and the jacobian of a point of the search space.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct FitProblem {
    problem: sbmlsim_fit::Problem,
}

impl FitProblem {
    pub(crate) fn new(problem: sbmlsim_fit::Problem) -> FitProblem {
        FitProblem { problem }
    }

    /// The point of the search space a call was given, of the parameters of the fit.
    fn point(&self, x: Vec<f64>) -> PyResult<Vec<f64>> {
        let wanted = self.problem.parameters().len();
        if x.len() != wanted {
            return Err(PyValueError::new_err(format!(
                "the fit has {wanted} parameters and the point has {} values",
                x.len()
            )));
        }
        if let Some(value) = x.iter().find(|value| !value.is_finite()) {
            return Err(PyValueError::new_err(format!(
                "the point of the search space has the value {value}, which is not finite"
            )));
        }
        Ok(x)
    }
}

#[pymethods]
impl FitProblem {
    /// The ids of the parameters an optimizer searches, in the order of the point.
    #[getter]
    fn parameters(&self) -> Vec<String> {
        self.problem
            .parameters()
            .iter()
            .map(|parameter| parameter.id.clone())
            .collect()
    }

    /// The number of residuals of one evaluation, the measurements of every curve.
    #[getter]
    fn n_residuals(&self) -> usize {
        self.problem.n_residuals()
    }

    /// The threads of the batch of an evaluation.
    #[getter]
    fn threads(&self) -> usize {
        self.problem.threads()
    }

    /// The start of the fit, in the space an optimizer searches.
    #[getter]
    fn start(&self) -> Vec<f64> {
        self.problem.start()
    }

    /// The bounds of the parameters in that space, as `(lower, upper)`.
    #[getter]
    fn bounds(&self) -> (Vec<f64>, Vec<f64>) {
        self.problem.bounds()
    }

    /// The values of the parameters at a point of the search space, on the linear scale.
    fn values(&self, x: Vec<f64>) -> PyResult<Vec<f64>> {
        Ok(self.problem.parameter_values(&self.point(x)?))
    }

    /// The relative step the jacobian of the parameter `at` takes at a point.
    fn step(&self, at: usize, x: Vec<f64>) -> PyResult<f64> {
        let x = self.point(x)?;
        if at >= self.problem.parameters().len() {
            return Err(PyValueError::new_err(format!(
                "the fit has {} parameters, so {at} is none of them",
                self.problem.parameters().len()
            )));
        }
        Ok(self.problem.step(at, &x))
    }

    /// The residuals at a point of the search space, one per measurement, as one batch of
    /// the engine. The residuals of a condition whose integration failed are the large
    /// finite number a failed evaluation is scored with.
    fn residuals<'py>(&self, py: Python<'py>, x: Vec<f64>) -> PyResult<Bound<'py, PyArray1<f64>>> {
        let x = self.point(x)?;
        let residuals = py
            .detach(|| self.problem.residuals(&x))
            .map_err(fit_error)?;
        Ok(PyArray1::from_vec(py, residuals))
    }

    /// Half the sum of the squares of those residuals.
    fn cost(&self, py: Python<'_>, x: Vec<f64>) -> PyResult<f64> {
        let x = self.point(x)?;
        py.detach(|| self.problem.cost(&x)).map_err(fit_error)
    }

    /// The jacobian of the residuals at a point, `(n_residuals, parameters)`.
    ///
    /// The whole jacobian is one batch of `(parameters + 1) * conditions` simulations, which
    /// fans out over the threads of the engine at once instead of once per column.
    fn jacobian<'py>(&self, py: Python<'py>, x: Vec<f64>) -> PyResult<Bound<'py, PyArray2<f64>>> {
        let x = self.point(x)?;
        let values = py.detach(|| self.problem.jacobian(&x)).map_err(fit_error)?;
        let shape = (self.problem.n_residuals(), self.problem.parameters().len());
        let jacobian = numpy::ndarray::Array2::from_shape_vec(shape, values).map_err(|error| {
            PyValueError::new_err(format!("the jacobian of the fit has no shape: {error}"))
        })?;
        Ok(PyArray2::from_owned_array(py, jacobian))
    }

    fn __repr__(&self) -> String {
        format!(
            "FitProblem(parameters={}, residuals={}, threads={})",
            self.problem.parameters().len(),
            self.problem.n_residuals(),
            self.problem.threads()
        )
    }
}
