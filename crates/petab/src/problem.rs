//! A problem of PEtab v2 as its files give it: the YAML file, the models and the tables.
//!
//! [`PetabProblem::read`] reads the files through [`Files`], so the reader itself touches no
//! file system: a host of a browser hands over the contents of the files as bytes
//! ([`InMemory`]), and a native host reads them from a directory ([`Directory`]). The tables
//! are read and checked here, the math of their formulas is read by the [`crate::plan`], which
//! translates the problem into simulations of the engine.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::Deserialize;

use crate::PetabError;
use crate::likelihood::NoiseDistribution;
use crate::tables::{Table, number};

/// Where the files of a problem come from, by their names relative to the YAML file of the
/// problem.
pub trait Files {
    /// The content of a file.
    ///
    /// # Errors
    ///
    /// Why the file cannot be read.
    fn read(&self, name: &str) -> Result<Vec<u8>, String>;
}

/// The files of a problem in memory, by their names, e.g. the files a page of a browser was
/// given.
pub type InMemory = HashMap<String, Vec<u8>>;

impl Files for HashMap<String, Vec<u8>> {
    fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        self.get(name)
            .or_else(|| self.get(name.trim_start_matches("./")))
            .cloned()
            .ok_or_else(|| format!("there is no file '{name}'"))
    }
}

/// The files of a problem in a directory, the one of its YAML file.
#[derive(Debug, Clone)]
pub struct Directory(pub PathBuf);

impl Files for Directory {
    fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let path = self.0.join(name);
        std::fs::read(&path).map_err(|err| format!("'{}': {err}", path.display()))
    }
}

/// A model of a problem.
#[derive(Debug, Clone)]
pub struct ModelFile {
    /// The id of the model in the problem.
    pub id: String,
    /// The name of its file.
    pub location: String,
    /// The text of the SBML document; a composed model (the package `comp`) flattened with
    /// the documents of its external model definitions, which are named relative to it.
    pub sbml: String,
}

/// A row of the condition table: a change of one target.
#[derive(Debug, Clone)]
pub struct ConditionChange {
    /// The id of the condition the change belongs to.
    pub condition: String,
    /// The id of the entity the change sets.
    pub target: String,
    /// The math of the value it sets.
    pub value: String,
}

/// A row of the experiment table: a condition which becomes active at a time.
#[derive(Debug, Clone)]
pub struct Period {
    /// The id of the experiment.
    pub experiment: String,
    /// The time the condition becomes active, `-inf` for the steady state before the start.
    pub time: f64,
    /// The condition, `None` for a period of no change.
    pub condition: Option<String>,
}

/// A row of the observable table.
#[derive(Debug, Clone)]
pub struct Observable {
    /// The id.
    pub id: String,
    /// The math of the observable.
    pub formula: String,
    /// The math of the scale of its noise.
    pub noise_formula: String,
    /// The distribution of its noise, normal when the table does not give one.
    pub distribution: NoiseDistribution,
    /// The placeholders of the observable, which a measurement gives values.
    pub placeholders: Vec<String>,
    /// The placeholders of the noise, which a measurement gives values.
    pub noise_placeholders: Vec<String>,
}

/// A row of the measurement table.
#[derive(Debug, Clone)]
pub struct Measurement {
    /// The model which simulates the measurement, `None` for a problem of one model.
    pub model: Option<String>,
    /// The id of the observable.
    pub observable: String,
    /// The experiment, `None` for the model as it is.
    pub experiment: Option<String>,
    /// The time, `inf` for the steady state after the experiment.
    pub time: f64,
    /// The measured value.
    pub value: f64,
    /// The values of the placeholders of the observable, a number or the id of a parameter
    /// each.
    pub observable_parameters: Vec<String>,
    /// The values of the placeholders of the noise.
    pub noise_parameters: Vec<String>,
}

/// The prior of a parameter as the parameter table gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct PriorSpec {
    /// The name of the distribution, e.g. `normal`.
    pub distribution: String,
    /// The parameters of the distribution.
    pub parameters: Vec<f64>,
}

/// A row of the parameter table.
#[derive(Debug, Clone)]
pub struct ParameterRow {
    /// The id.
    pub id: String,
    /// The lower bound, `-inf` when the table gives none.
    pub lower: f64,
    /// The upper bound, `inf` when the table gives none.
    pub upper: f64,
    /// The nominal value.
    pub nominal: Option<f64>,
    /// Whether the parameter is estimated.
    pub estimate: bool,
    /// Its prior.
    pub prior: Option<PriorSpec>,
}

/// A row of the mapping table.
#[derive(Debug, Clone)]
pub struct Mapping {
    /// The id of PEtab.
    pub petab: String,
    /// The id of the entity of a model it stands for, `None` for a row of an annotation.
    pub model: Option<String>,
}

/// An extension of the problem.
#[derive(Debug, Clone)]
pub struct Extension {
    /// Its version.
    pub version: String,
    /// Whether the problem cannot be read without it.
    pub required: bool,
    /// The whole block of the extension.
    pub content: serde_json::Value,
}

/// A problem of PEtab v2, its files read and its tables checked.
#[derive(Debug, Clone)]
pub struct PetabProblem {
    /// The id of the problem.
    pub id: Option<String>,
    /// The models, in the order of the YAML file.
    pub models: Vec<ModelFile>,
    /// The rows of the condition tables.
    pub conditions: Vec<ConditionChange>,
    /// The rows of the experiment tables.
    pub periods: Vec<Period>,
    /// The rows of the observable tables.
    pub observables: Vec<Observable>,
    /// The rows of the measurement tables, in their order.
    pub measurements: Vec<Measurement>,
    /// The rows of the parameter tables.
    pub parameters: Vec<ParameterRow>,
    /// The rows of the mapping tables.
    pub mappings: Vec<Mapping>,
    /// The extensions, by their names.
    pub extensions: BTreeMap<String, Extension>,
}

/// The YAML file of a problem.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    format_version: serde_json::Value,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    parameter_files: Vec<String>,
    #[serde(default)]
    model_files: BTreeMap<String, YamlModel>,
    #[serde(default)]
    measurement_files: Vec<String>,
    #[serde(default)]
    condition_files: Vec<String>,
    #[serde(default)]
    experiment_files: Vec<String>,
    #[serde(default)]
    observable_files: Vec<String>,
    #[serde(default)]
    mapping_files: Vec<String>,
    #[serde(default)]
    extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YamlModel {
    location: String,
    language: String,
}

fn text(files: &dyn Files, name: &str) -> Result<String, PetabError> {
    let bytes = files.read(name).map_err(PetabError::File)?;
    String::from_utf8(bytes).map_err(|_| PetabError::File(format!("'{name}' is no UTF-8")))
}

fn tables(files: &dyn Files, names: &[String]) -> Result<Table, PetabError> {
    let mut table = Table::default();
    for name in names {
        let read = Table::read(name, &text(files, name)?).map_err(PetabError::Table)?;
        table.extend(read);
    }
    Ok(table)
}

/// A list of ids of a cell, separated by semicolons, empty for an empty cell.
fn list(cell: &str) -> Vec<String> {
    if cell.trim().is_empty() {
        return Vec::new();
    }
    cell.split(';').map(|id| id.trim().to_string()).collect()
}

fn table_error(table: &Table, row: usize, message: String) -> PetabError {
    PetabError::Table(format!("the row {} of '{}' {message}", row + 1, table.file))
}

impl PetabProblem {
    /// Reads a problem: its YAML file by its name among the files, and every file it names
    /// relative to it.
    ///
    /// # Errors
    ///
    /// A file which is missing or no file of PEtab v2, a model of a language other than SBML
    /// and a table which lacks a required column or holds a cell which is no value of it.
    pub fn read(yaml: &str, files: &dyn Files) -> Result<PetabProblem, PetabError> {
        let content: Yaml = serde_norway::from_str(&text(files, yaml)?).map_err(|err| {
            PetabError::File(format!("'{yaml}' is no problem of PEtab v2: {err}"))
        })?;
        let version = match &content.format_version {
            serde_json::Value::String(version) => version.clone(),
            other => other.to_string(),
        };
        if !(version == "2" || version.starts_with("2.")) {
            return Err(PetabError::Unsupported(format!(
                "the format version {version}: the reader reads PEtab 2.0.0, convert a problem of \
                 PEtab 1 first (petab.v2.petab1to2)"
            )));
        }
        // a file is named relative to the YAML file
        let base = match yaml.rfind('/') {
            Some(k) => &yaml[..=k],
            None => "",
        };
        let relative = |names: &[String]| -> Vec<String> {
            names.iter().map(|name| format!("{base}{name}")).collect()
        };

        let mut models = Vec::new();
        for (id, model) in &content.model_files {
            if !model.language.eq_ignore_ascii_case("sbml") {
                return Err(PetabError::Unsupported(format!(
                    "the model '{id}' of the language '{}': the engine simulates SBML",
                    model.language
                )));
            }
            let path = format!("{base}{}", model.location);
            // the external documents of a composed model are named relative to the model, and
            // read through the files of the problem
            let directory = match path.rfind('/') {
                Some(k) => &path[..=k],
                None => "",
            };
            let sources = |source: &str| -> Result<String, String> {
                text(files, &format!("{directory}{source}")).map_err(|err| err.to_string())
            };
            let sbml =
                sbmlsim_sbml::flatten_comp(&text(files, &path)?, &sources).map_err(|err| {
                    PetabError::Compile {
                        model: id.clone(),
                        message: err.to_string(),
                    }
                })?;
            models.push(ModelFile {
                id: id.clone(),
                location: model.location.clone(),
                sbml,
            });
        }
        if models.is_empty() {
            return Err(PetabError::File(format!("'{yaml}' names no model")));
        }

        let mut extensions = BTreeMap::new();
        for (name, block) in content.extensions {
            let version = match block.get("version") {
                Some(serde_json::Value::String(version)) => version.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            let required = block
                .get("required")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            extensions.insert(
                name,
                Extension {
                    version,
                    required,
                    content: block,
                },
            );
        }

        Ok(PetabProblem {
            id: content.id,
            models,
            conditions: conditions(&tables(files, &relative(&content.condition_files))?)?,
            periods: periods(&tables(files, &relative(&content.experiment_files))?)?,
            observables: observables(&tables(files, &relative(&content.observable_files))?)?,
            measurements: measurements(&tables(files, &relative(&content.measurement_files))?)?,
            parameters: parameters(&tables(files, &relative(&content.parameter_files))?)?,
            mappings: mappings(&tables(files, &relative(&content.mapping_files))?)?,
            extensions,
        })
    }

    /// Reads a problem from its YAML file on disk, with the files it names next to it.
    ///
    /// # Errors
    ///
    /// See [`PetabProblem::read`].
    pub fn from_path(yaml: impl Into<PathBuf>) -> Result<PetabProblem, PetabError> {
        let yaml = yaml.into();
        let directory = yaml
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = yaml
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| PetabError::File(format!("'{}' is no file", yaml.display())))?;
        PetabProblem::read(name, &Directory(directory))
    }
}

fn conditions(table: &Table) -> Result<Vec<ConditionChange>, PetabError> {
    if table.header.is_empty() {
        return Ok(Vec::new());
    }
    let (id, target, value) = (
        table.required("conditionId").map_err(PetabError::Table)?,
        table.required("targetId").map_err(PetabError::Table)?,
        table.required("targetValue").map_err(PetabError::Table)?,
    );
    table
        .rows
        .iter()
        .enumerate()
        .map(|(k, row)| {
            if row[id].is_empty() || row[target].is_empty() || row[value].is_empty() {
                return Err(table_error(
                    table,
                    k,
                    "has no conditionId, targetId or targetValue".to_string(),
                ));
            }
            Ok(ConditionChange {
                condition: row[id].clone(),
                target: row[target].clone(),
                value: row[value].clone(),
            })
        })
        .collect()
}

fn periods(table: &Table) -> Result<Vec<Period>, PetabError> {
    if table.header.is_empty() {
        return Ok(Vec::new());
    }
    let (id, time, condition) = (
        table.required("experimentId").map_err(PetabError::Table)?,
        table.required("time").map_err(PetabError::Table)?,
        table.required("conditionId").map_err(PetabError::Table)?,
    );
    table
        .rows
        .iter()
        .enumerate()
        .map(|(k, row)| {
            let at = number(&row[time])
                .filter(|t| !t.is_nan() && *t != f64::INFINITY)
                .ok_or_else(|| {
                    table_error(
                        table,
                        k,
                        format!("has the time '{}', which is no number or -inf", row[time]),
                    )
                })?;
            if row[id].is_empty() {
                return Err(table_error(table, k, "has no experimentId".to_string()));
            }
            Ok(Period {
                experiment: row[id].clone(),
                time: at,
                condition: (!row[condition].is_empty()).then(|| row[condition].clone()),
            })
        })
        .collect()
}

fn observables(table: &Table) -> Result<Vec<Observable>, PetabError> {
    let id = table.required("observableId").map_err(PetabError::Table)?;
    let formula = table
        .required("observableFormula")
        .map_err(PetabError::Table)?;
    let noise = table.required("noiseFormula").map_err(PetabError::Table)?;
    let distribution = table.column("noiseDistribution");
    let placeholders = table.column("observablePlaceholders");
    let noise_placeholders = table.column("noisePlaceholders");
    table
        .rows
        .iter()
        .enumerate()
        .map(|(k, row)| {
            let cell = |column: Option<usize>| column.map_or("", |c| row[c].as_str());
            let distribution = match cell(distribution) {
                "" => NoiseDistribution::Normal,
                name => NoiseDistribution::from_name(name).ok_or_else(|| {
                    table_error(
                        table,
                        k,
                        format!("has the noise distribution '{name}', which is none of PEtab v2"),
                    )
                })?,
            };
            if row[id].is_empty() || row[formula].is_empty() || row[noise].is_empty() {
                return Err(table_error(
                    table,
                    k,
                    "has no observableId, observableFormula or noiseFormula".to_string(),
                ));
            }
            Ok(Observable {
                id: row[id].clone(),
                formula: row[formula].clone(),
                noise_formula: row[noise].clone(),
                distribution,
                placeholders: list(cell(placeholders)),
                noise_placeholders: list(cell(noise_placeholders)),
            })
        })
        .collect()
}

fn measurements(table: &Table) -> Result<Vec<Measurement>, PetabError> {
    let observable = table.required("observableId").map_err(PetabError::Table)?;
    let time = table.required("time").map_err(PetabError::Table)?;
    let value = table.required("measurement").map_err(PetabError::Table)?;
    let experiment = table.column("experimentId");
    let model = table.column("modelId");
    let observable_parameters = table.column("observableParameters");
    let noise_parameters = table.column("noiseParameters");
    table
        .rows
        .iter()
        .enumerate()
        .map(|(k, row)| {
            let cell = |column: Option<usize>| column.map_or("", |c| row[c].as_str());
            let optional = |column: Option<usize>| {
                let text = cell(column);
                (!text.is_empty()).then(|| text.to_string())
            };
            let at = number(&row[time])
                .filter(|t| !t.is_nan() && *t != f64::NEG_INFINITY)
                .ok_or_else(|| {
                    table_error(
                        table,
                        k,
                        format!("has the time '{}', which is no number or inf", row[time]),
                    )
                })?;
            let measured = number(&row[value]).ok_or_else(|| {
                table_error(
                    table,
                    k,
                    format!("has the measurement '{}', which is no number", row[value]),
                )
            })?;
            Ok(Measurement {
                model: optional(model),
                observable: row[observable].clone(),
                experiment: optional(experiment),
                time: at,
                value: measured,
                observable_parameters: list(cell(observable_parameters)),
                noise_parameters: list(cell(noise_parameters)),
            })
        })
        .collect()
}

fn parameters(table: &Table) -> Result<Vec<ParameterRow>, PetabError> {
    let id = table.required("parameterId").map_err(PetabError::Table)?;
    let estimate = table.required("estimate").map_err(PetabError::Table)?;
    let lower = table.column("lowerBound");
    let upper = table.column("upperBound");
    let nominal = table.column("nominalValue");
    let distribution = table.column("priorDistribution");
    let prior_parameters = table.column("priorParameters");
    table
        .rows
        .iter()
        .enumerate()
        .map(|(k, row)| {
            let cell = |column: Option<usize>| column.map_or("", |c| row[c].trim());
            let value = |column: Option<usize>, name: &str| -> Result<Option<f64>, PetabError> {
                match cell(column) {
                    "" => Ok(None),
                    text => number(text).map(Some).ok_or_else(|| {
                        table_error(
                            table,
                            k,
                            format!("has the {name} '{text}', which is no number"),
                        )
                    }),
                }
            };
            let estimated = match row[estimate].trim().to_ascii_lowercase().as_str() {
                "true" => true,
                "false" => false,
                other => {
                    return Err(table_error(
                        table,
                        k,
                        format!("has the estimate '{other}', which is neither true nor false"),
                    ));
                }
            };
            let prior = match cell(distribution) {
                "" => None,
                name => {
                    let parameters = cell(prior_parameters)
                        .split(';')
                        .filter(|text| !text.trim().is_empty())
                        .map(|text| {
                            number(text).ok_or_else(|| {
                                table_error(
                                    table,
                                    k,
                                    format!("has the prior parameter '{text}', which is no number"),
                                )
                            })
                        })
                        .collect::<Result<Vec<f64>, PetabError>>()?;
                    Some(PriorSpec {
                        distribution: name.to_string(),
                        parameters,
                    })
                }
            };
            let row = ParameterRow {
                id: row[id].trim().to_string(),
                lower: value(lower, "lowerBound")?.unwrap_or(f64::NEG_INFINITY),
                upper: value(upper, "upperBound")?.unwrap_or(f64::INFINITY),
                nominal: value(nominal, "nominalValue")?,
                estimate: estimated,
                prior,
            };
            if row.id.is_empty() {
                return Err(table_error(table, k, "has no parameterId".to_string()));
            }
            if !row.estimate && row.nominal.is_none() {
                return Err(table_error(
                    table,
                    k,
                    format!("fixes the parameter '{}' without a nominalValue", row.id),
                ));
            }
            if row.estimate && (row.lower.is_nan() || row.upper.is_nan() || row.lower > row.upper) {
                return Err(table_error(
                    table,
                    k,
                    format!(
                        "estimates '{}' between the bounds {} and {}",
                        row.id, row.lower, row.upper
                    ),
                ));
            }
            Ok(row)
        })
        .collect()
}

fn mappings(table: &Table) -> Result<Vec<Mapping>, PetabError> {
    if table.header.is_empty() {
        return Ok(Vec::new());
    }
    let petab = table.required("petabEntityId").map_err(PetabError::Table)?;
    let model = table.required("modelEntityId").map_err(PetabError::Table)?;
    Ok(table
        .rows
        .iter()
        .map(|row| Mapping {
            petab: row[petab].clone(),
            model: (!row[model].is_empty()).then(|| row[model].clone()),
        })
        .collect())
}
