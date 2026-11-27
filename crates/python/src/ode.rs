//! Thin native bindings around the portable equation and source-generation crate.
use pyo3::{exceptions::PyValueError, prelude::*};
use sbmlsim_ode::{Links, OdeSystem, RenderOptions};

#[derive(serde::Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct LinkInput {
    symbols: std::collections::HashMap<String, String>,
    locals: Vec<LocalLink>,
    metadata: Vec<MetaLink>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalLink {
    reaction: String,
    index: usize,
    link: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MetaLink {
    kind: String,
    metaid: String,
    link: String,
}
impl From<LinkInput> for Links {
    fn from(input: LinkInput) -> Self {
        Self {
            symbols: input.symbols,
            locals: input
                .locals
                .into_iter()
                .map(|v| ((v.reaction, v.index), v.link))
                .collect(),
            metadata: input
                .metadata
                .into_iter()
                .map(|v| ((v.kind, v.metaid), v.link))
                .collect(),
        }
    }
}

/// An immutable owned model; parsing and rendering release the Python interpreter.
#[pyclass(name = "_EquationSystem", module = "sbmlsim_core._core", frozen)]
pub struct EquationSystem {
    system: OdeSystem,
}
#[pymethods]
impl EquationSystem {
    #[new]
    #[pyo3(signature=(source,documents=None,location="./model.xml",max_expanded_elements=10_000))]
    fn new(
        py: Python<'_>,
        source: Vec<u8>,
        documents: Option<Vec<(String, Vec<u8>)>>,
        location: &str,
        max_expanded_elements: usize,
    ) -> PyResult<Self> {
        py.detach(|| {
            OdeSystem::read_with_sources(
                &source,
                location,
                documents.as_deref().unwrap_or(&[]),
                max_expanded_elements,
            )
            .map(|system| Self { system })
        })
        .map_err(PyValueError::new_err)
    }
    #[staticmethod]
    #[pyo3(signature=(path,max_expanded_elements=10_000))]
    fn from_path(py: Python<'_>, path: String, max_expanded_elements: usize) -> PyResult<Self> {
        py.detach(|| {
            OdeSystem::read_path(std::path::Path::new(&path), max_expanded_elements)
                .map(|system| Self { system })
        })
        .map_err(PyValueError::new_err)
    }
    #[getter]
    fn model_id(&self) -> Option<&str> {
        self.system.model_id()
    }
    fn render(&self, py: Python<'_>, fmt: String, options: String) -> PyResult<(String, String)> {
        let options: RenderOptions =
            serde_json::from_str(&options).map_err(|e| PyValueError::new_err(e.to_string()))?;
        py.detach(|| {
            self.system
                .render(&fmt, &options)
                .map(|file| (file.filename, file.content))
        })
        .map_err(PyValueError::new_err)
    }
    fn render_template(
        &self,
        py: Python<'_>,
        fmt: String,
        options: String,
        template: String,
    ) -> PyResult<String> {
        let options: RenderOptions =
            serde_json::from_str(&options).map_err(|e| PyValueError::new_err(e.to_string()))?;
        py.detach(|| {
            self.system
                .render_template(&fmt, &options, &template)
                .map(|file| file.content)
        })
        .map_err(PyValueError::new_err)
    }
    fn typeset_json(&self, py: Python<'_>, links: String) -> PyResult<String> {
        let links: LinkInput =
            serde_json::from_str(&links).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let links = Links::from(links);
        py.detach(|| {
            self.system
                .typeset(&links)
                .and_then(|system| serde_json::to_string(&system).map_err(|e| e.to_string()))
        })
        .map_err(PyValueError::new_err)
    }
}
