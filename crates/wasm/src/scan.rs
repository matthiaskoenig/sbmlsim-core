//! The scan of sbmlsim in a browser: the planner of `sbmlsim-scan` in `compiler.wasm`, its
//! simulations run by the engine of the host, `js/scan.mjs` the caller.
//!
//! The planner is a state machine which simulates nothing itself (see `sbmlsim-scan`): the
//! host opens a scan of a request `scan/1` against the description of a compiled model (or the
//! descriptions of the models of a dimension of models by their keys), asks
//! for its rounds, simulates the requests of `simulation/1` of a round (or computes the
//! observables of the host of a round), tells their answers, and takes the result
//! `scanresult/1` once the scan is done:
//!
//! 1. [`open`] plans the scan and gives a handle and the number of its points,
//! 2. [`ask`] gives the next round: the requests to simulate and their model, the native time
//!    and columns of the observables of the host, or the end,
//! 3. [`tell`] takes the tables (or the errors) of the requests of a round,
//! 4. [`observed`] takes the values of the observables of the host of a round,
//! 5. [`result`] gives the result, and [`close`] drops the scan.
//!
//! Every array of numbers crosses as the little-endian bytes of its `f64`s in a buffer of its
//! own next to the JSON (the tables of the engine, the time and the columns of the observables
//! of the host, their values, and the values of the variables of the result), so `NaN` and the
//! infinities cross as the values they are and nothing large is written into JSON and parsed
//! again. The few numbers which stay in the JSON of the result (the coordinates, the times of
//! the changes) are written as `scanresult/1` writes them: a number, `null` for `NaN` and the
//! strings `"inf"` and `"-inf"`.
//!
//! The C ABI carries the same calls as `scan_open`, `scan_ask`, `scan_tell`, `scan_observed`,
//! `scan_result` and `scan_close`. A call gives 1 with its answer, or 0 with the error of the
//! scan as the JSON `{"kind": ..., "message": ..., "point": ...}` (the kinds of
//! `sbmlsim_scan::ScanErrorKind`: `request`, `units`, `unsupported` and `point`).

use std::cell::RefCell;

use sbmlsim_abi::{EngineError, ModelDescription, SimulationResult};
use sbmlsim_scan::{
    Ask, HostError, HostObserved, Ordered, PointError, Scan, ScanError, ScanRequest,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{bytes, give};

thread_local! {
    /// The open scans, by their handle, their index.
    static SCANS: RefCell<Vec<Option<Scan>>> = const { RefCell::new(Vec::new()) };
}

/// The JSON of the error of a scan.
fn error_json(error: &ScanError) -> String {
    json!({
        "kind": error.kind,
        "message": error.message,
        "point": error.point,
    })
    .to_string()
}

/// The answer of a call: the JSON (and the bytes of its numbers), or the error of the scan.
type Answer = Result<(String, Vec<u8>), ScanError>;

/// Runs `work` on an open scan.
fn with_scan<T>(
    handle: usize,
    work: impl FnOnce(&mut Scan) -> Result<T, ScanError>,
) -> Result<T, ScanError> {
    SCANS.with(|scans| {
        let mut scans = scans.borrow_mut();
        let scan = scans
            .get_mut(handle)
            .and_then(Option::as_mut)
            .ok_or_else(|| ScanError::request(format!("There is no open scan {handle}.")))?;
        work(scan)
    })
}

/// The little-endian bytes of numbers, appended.
fn put(out: &mut Vec<u8>, values: &[f64]) {
    out.reserve(8 * values.len());
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// The numbers of the little-endian bytes of `f64`s.
fn numbers_of(values: &[u8]) -> Result<Vec<f64>, ScanError> {
    let (chunks, rest) = values.as_chunks::<8>();
    if !rest.is_empty() {
        return Err(ScanError::request(format!(
            "The values are {} bytes, no whole number of f64.",
            values.len()
        )));
    }
    Ok(chunks.iter().map(|b| f64::from_le_bytes(*b)).collect())
}

/// The progress of a scan as JSON, `[done, total]`.
fn progress(scan: &Scan) -> Value {
    let (done, total) = scan.progress();
    json!([done, total])
}

/// Plans a scan of the request `scan/1` (its JSON) against the description of a compiled model
/// (the JSON of `model.json`, empty for none), or against the descriptions of the models of a
/// dimension of models (the JSON `{key: description, ...}`, empty for none), and gives the JSON
/// `{"handle": h, "points": n}`, the handle the other calls take and the number of the points
/// of the block which run.
///
/// # Errors
///
/// A request or a description which is no such JSON, and a request which does not fit itself
/// or its models.
pub fn open(request: &[u8], description: &[u8], models: &[u8]) -> Answer {
    let request: ScanRequest = serde_json::from_slice(request)
        .map_err(|err| ScanError::request(format!("The request is no request of scan/1: {err}")))?;
    let read = |bytes: &[u8]| -> Result<ModelDescription, ScanError> {
        serde_json::from_slice(bytes).map_err(|err| {
            ScanError::request(format!(
                "The description is no description of model/2: {err}"
            ))
        })
    };
    let description = (!description.is_empty())
        .then(|| read(description))
        .transpose()?;
    let models: Vec<(String, ModelDescription)> = if models.is_empty() {
        Vec::new()
    } else {
        let models: Ordered<ModelDescription> = serde_json::from_slice(models).map_err(|err| {
            ScanError::request(format!(
                "The models are no descriptions of model/2 by their keys: {err}"
            ))
        })?;
        models.0
    };
    let scan = Scan::open_with_models(request, description, models)?;
    let points = scan.progress().1;
    let handle = SCANS.with(|scans| {
        let mut scans = scans.borrow_mut();
        scans.push(Some(scan));
        scans.len() - 1
    });
    Ok((
        json!({ "handle": handle, "points": points }).to_string(),
        Vec::new(),
    ))
}

/// The next round of an open scan as JSON, every answer with the `progress` `[done, total]`:
///
/// - `{"simulate": [request, ...], "model": key, "finished": [key, ...]}`, the requests of
///   `simulation/1` of a batch on the model of the key in the dimension of models (`null` for
///   the model of the run), which [`tell`] answers in their order, and the keys of the models
///   whose points have all run (`Scan::finished_models`), whose engines a host may drop;
/// - `{"observe": {"observables": [id, ...], "points": [{"index": i, "time": n, "columns":
///   [[symbol, n], ...]}, ...], "scalars": [symbol, ...]}}`, the observables of the host of the
///   points of a chunk, which [`observed`] answers: per point its `n` finite time points and
///   then every column of its `n` values (or one value of a value per simulation, the symbols
///   of `scalars`) are the bytes of the answer, in that order;
/// - `{"done": true}`: the scan is done, or failed, which [`result`] says.
///
/// # Errors
///
/// A handle of no open scan.
pub fn ask(handle: usize) -> Answer {
    with_scan(handle, |scan| {
        let mut values = Vec::new();
        let answer = match scan.ask() {
            Ask::Simulate { model, requests } => json!({
                "simulate": requests,
                "model": model,
                "finished": scan.finished_models(),
            }),
            Ask::Observe(question) => {
                let points: Vec<Value> = question
                    .points
                    .iter()
                    .map(|point| {
                        put(&mut values, &point.time);
                        let columns: Vec<Value> = point
                            .columns
                            .iter()
                            .map(|(symbol, column)| {
                                put(&mut values, column);
                                json!([symbol, column.len()])
                            })
                            .collect();
                        json!({
                            "index": point.index,
                            "time": point.time.len(),
                            "columns": columns,
                        })
                    })
                    .collect();
                json!({
                    "observe": {
                        "observables": question.observables,
                        "points": points,
                        "scalars": question.scalars,
                    }
                })
            }
            Ask::Done => json!({ "done": true }),
        };
        let mut answer = answer;
        answer["progress"] = progress(scan);
        Ok((answer.to_string(), values))
    })
}

/// The answer of a request as the host gives it: the shape of the table of `simulation/1`,
/// whose values are in the buffer of the values, or the error of the engine, which is read
/// back into the error of the engine the native hosts answer with, so a failed point has the
/// words of python and of the command line.
#[derive(Deserialize)]
#[serde(untagged)]
enum HostResult {
    Table {
        columns: Vec<String>,
        n_rows: usize,
    },
    Failure {
        /// The message of the engine without the words of its kind (the `detail` of an
        /// `EngineError` of the loader).
        error: String,
        /// The kind of the error of the engine: `request`, `integration` (with the `flag` of
        /// CVODE and the `time` it reached), `unsupported`, or anything else for an internal
        /// failure.
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        flag: Option<i32>,
        #[serde(default)]
        time: Option<f64>,
    },
}

/// What [`tell`] reads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TellRequest {
    results: Vec<HostResult>,
}

/// Takes the answers of the round of simulations of an open scan: the JSON `{"results":
/// [...]}` of the shape `{"columns": [...], "n_rows": n}` of a table or `{"error": message,
/// "kind": kind, "flag": f, "time": t}` per request (the error of the engine, see
/// `HostResult`), and the values of the tables one after the
/// other, each row major, as the bytes of their `f64`s; gives `{"progress": [done, total]}`.
///
/// # Errors
///
/// A handle of no open scan, an input which is no such JSON, values which are not those of the
/// tables, answers which do not fit the round, and the failure of the scan (a point of a scan
/// of `on_error: raise`).
pub fn tell(handle: usize, input: &[u8], values: &[u8]) -> Answer {
    let input: TellRequest = serde_json::from_slice(input)
        .map_err(|err| ScanError::request(format!("The answers are no such JSON: {err}")))?;
    let mut values = numbers_of(values)?.into_iter();
    let mut answers = Vec::with_capacity(input.results.len());
    for result in input.results {
        answers.push(match result {
            HostResult::Table { columns, n_rows } => {
                let n = n_rows
                    .checked_mul(columns.len())
                    .ok_or_else(|| ScanError::request("A table of more values than an address."))?;
                let table: Vec<f64> = values.by_ref().take(n).collect();
                if table.len() != n {
                    return Err(ScanError::request(
                        "The values end before the tables of the answers.",
                    ));
                }
                Ok(SimulationResult {
                    columns,
                    n_rows,
                    values: table,
                    sensitivities: None,
                    local_errors: None,
                })
            }
            HostResult::Failure {
                error,
                kind,
                flag,
                time,
            } => Err(PointError::from(match kind.as_deref() {
                Some("request") => EngineError::Request(error),
                Some("integration") => EngineError::Integration {
                    flag: flag.unwrap_or(0),
                    time: time.unwrap_or(f64::NAN),
                    message: error,
                },
                Some("unsupported") => EngineError::Unsupported(error),
                _ => EngineError::Internal(error),
            })),
        });
    }
    if values.next().is_some() {
        return Err(ScanError::request(
            "The values are more than the tables of the answers.",
        ));
    }
    with_scan(handle, |scan| {
        scan.tell(answers)?;
        Ok((
            json!({ "progress": progress(scan) }).to_string(),
            Vec::new(),
        ))
    })
}

/// What [`observed`] reads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedRequest {
    /// Per observable its id and the number of the values of every point, in the order of the
    /// points of the question.
    observables: Vec<(String, Vec<usize>)>,
    #[serde(default)]
    errors: Vec<HostError>,
}

/// Takes the values of the round of the observables of the host of an open scan: the JSON
/// `{"observables": [[id, [n of every point]], ...], "errors": [{"observable": id, "index": i,
/// "message": m}, ...]}` and the values, observable after observable and point after point, as
/// the bytes of their `f64`s; gives `{"progress": [done, total]}`.
///
/// # Errors
///
/// A handle of no open scan, an input which is no such JSON, values which are not those the
/// input counts, an answer which does not fit the question, and the failure of the scan.
pub fn observed(handle: usize, input: &[u8], values: &[u8]) -> Answer {
    let input: ObservedRequest = serde_json::from_slice(input)
        .map_err(|err| ScanError::request(format!("The answer is no such JSON: {err}")))?;
    let mut values = numbers_of(values)?.into_iter();
    let mut observed = HostObserved {
        values: Default::default(),
        errors: input.errors,
    };
    for (id, counts) in input.observables {
        let mut points = Vec::with_capacity(counts.len());
        for n in counts {
            let point: Vec<f64> = values.by_ref().take(n).collect();
            if point.len() != n {
                return Err(ScanError::request(
                    "The values end before the values the answer counts.",
                ));
            }
            points.push(point);
        }
        observed.values.insert(id, points);
    }
    if values.next().is_some() {
        return Err(ScanError::request(
            "The values are more than the values the answer counts.",
        ));
    }
    with_scan(handle, |scan| {
        scan.tell_observed(observed)?;
        Ok((
            json!({ "progress": progress(scan) }).to_string(),
            Vec::new(),
        ))
    })
}

/// The result `scanresult/1` of an open scan which is done: its JSON, every variable with no
/// values, and the values of the variables one after the other (each of the product of its
/// `shape` values, flat in C order) as the bytes of their `f64`s. A scan gives its result
/// once; a scan which is not done stays open.
///
/// # Errors
///
/// A handle of no open scan, a scan which is not done, and the failure of the scan.
pub fn result(handle: usize) -> Answer {
    let scan = SCANS.with(|scans| {
        let mut scans = scans.borrow_mut();
        let slot = scans
            .get_mut(handle)
            .filter(|slot| slot.is_some())
            .ok_or_else(|| ScanError::request(format!("There is no open scan {handle}.")))?;
        slot.as_ref().expect("an open scan").finished()?;
        Ok::<_, ScanError>(slot.take().expect("an open scan"))
    })?;
    let mut result = scan.result()?;
    let mut values = Vec::new();
    for variable in &mut result.variables {
        put(&mut values, &std::mem::take(&mut variable.values));
    }
    let text = serde_json::to_string(&result)
        .map_err(|err| ScanError::request(format!("The result is no JSON: {err}")))?;
    Ok((text, values))
}

/// Drops a scan; a handle of none is ignored.
pub fn close(handle: usize) {
    SCANS.with(|scans| {
        if let Some(slot) = scans.borrow_mut().get_mut(handle) {
            *slot = None;
        }
    });
}

/// Writes an answer to the four words at `out`: the JSON and the bytes of its numbers and 1,
/// or the JSON of the error, no bytes and 0.
///
/// # Safety
///
/// `out` points at four words of the caller.
unsafe fn deliver(answer: Answer, out: *mut usize) -> u32 {
    let (status, text, values) = match answer {
        Ok((text, values)) => (1, text, values),
        Err(error) => (0, error_json(&error), Vec::new()),
    };
    // SAFETY: the caller gives four words of its own memory
    unsafe {
        give(text.into_bytes(), out);
        give(values, out.add(2));
    }
    status
}

/// [`open`] of the C ABI: the request, the description and the models in three buffers (an
/// empty one, null and 0, for no description or no models), the answer in the four words at
/// `out` (the JSON and an empty buffer of numbers).
///
/// # Safety
///
/// `request`, `description` and `models` are buffers of [`crate::allocate`] of their lengths
/// (or null and 0), `out` four words of the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scan_open(
    request: *const u8,
    request_len: usize,
    description: *const u8,
    description_len: usize,
    models: *const u8,
    models_len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe {
        deliver(
            open(
                bytes(request, request_len),
                bytes(description, description_len),
                bytes(models, models_len),
            ),
            out,
        )
    }
}

/// [`ask`] of the C ABI, the answer in the four words at `out`.
///
/// # Safety
///
/// `out` is four words of the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scan_ask(handle: usize, out: *mut usize) -> u32 {
    // SAFETY: the buffer is the caller's, as documented
    unsafe { deliver(ask(handle), out) }
}

/// [`tell`] of the C ABI: the JSON at `pointer`, the bytes of the values at `values`, the
/// answer in the four words at `out`.
///
/// # Safety
///
/// `pointer` and `values` are buffers of [`crate::allocate`] of `len` and `values_len` bytes
/// (or null and 0), `out` four words of the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scan_tell(
    handle: usize,
    pointer: *const u8,
    len: usize,
    values: *const u8,
    values_len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe {
        deliver(
            tell(handle, bytes(pointer, len), bytes(values, values_len)),
            out,
        )
    }
}

/// [`observed`] of the C ABI, with the buffers of [`scan_tell`].
///
/// # Safety
///
/// As [`scan_tell`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scan_observed(
    handle: usize,
    pointer: *const u8,
    len: usize,
    values: *const u8,
    values_len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe {
        deliver(
            observed(handle, bytes(pointer, len), bytes(values, values_len)),
            out,
        )
    }
}

/// [`result`] of the C ABI, the answer in the four words at `out`.
///
/// # Safety
///
/// `out` is four words of the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scan_result(handle: usize, out: *mut usize) -> u32 {
    // SAFETY: the buffer is the caller's, as documented
    unsafe { deliver(result(handle), out) }
}

/// [`close`] of the C ABI.
#[unsafe(no_mangle)]
pub extern "C" fn scan_close(handle: usize) {
    close(handle);
}
