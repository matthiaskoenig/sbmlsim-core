//! The units of pint in a browser: the registry of `sbmlsim-units` in `compiler.wasm`, the
//! one the scan planner and python convert with.
//!
//! 1. [`open`] makes a registry of the builtin units and gives its handle,
//! 2. [`define`] defines a unit in it, from the JSON `{"name": ..., "expression": ...}`,
//! 3. [`parse`] gives the factor of an expression to the root units of pint and its
//!    dimensions, the JSON `{"factor": f, "dimensions": {"[length]": 3}}`,
//! 4. [`factor`] gives the factor which converts a value between two units, from the JSON
//!    `{"from": ..., "to": ...}`, as the JSON `{"factor": f}`,
//! 5. [`with_model_units`] makes a copy of a registry with the units of a model, from the JSON
//!    `{"units": {...}, "unit_ids": {...}}`, and gives its handle,
//! 6. [`unit_of`] gives the unit expression of an entity of the model of a registry, the JSON
//!    `{"unit": "mmol/l"}` (`null` without one),
//! 7. [`close`] drops the registry.
//!
//! The C ABI carries the same calls as `units_open`, `units_define`, `units_parse`,
//! `units_factor`, `units_with_model_units`, `units_unit_of` and `units_close`, every text as UTF-8 in the buffers of
//! [`crate::allocate`] and every answer, or the message of an error, in the two words of
//! `out` (the status is 1 for an answer and 0 for an error). A factor is a JSON number, which
//! `JSON.parse` reads back as the same double.

use std::cell::RefCell;
use std::collections::BTreeMap;

use sbmlsim_units::Registry;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{answer, bytes};

thread_local! {
    /// The open registries, by their handle, their index.
    static REGISTRIES: RefCell<Vec<Option<Registry>>> = const { RefCell::new(Vec::new()) };
}

/// What [`define`] reads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefineRequest {
    name: String,
    expression: String,
}

/// What [`factor`] reads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactorRequest {
    from: String,
    to: String,
}

/// What [`with_model_units`] reads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelUnitsRequest {
    #[serde(default)]
    units: BTreeMap<String, String>,
    #[serde(default)]
    unit_ids: BTreeMap<String, String>,
}

/// A factor as JSON: a number, or the strings `"Infinity"` and `"NaN"` of `Number` of
/// JavaScript for a factor out of the range of a double, which JSON has no number for.
fn number(value: f64) -> Value {
    if value.is_finite() {
        Value::from(value)
    } else if value.is_nan() {
        Value::from("NaN")
    } else if value > 0.0 {
        Value::from("Infinity")
    } else {
        Value::from("-Infinity")
    }
}

fn with_registry<T>(
    handle: usize,
    work: impl FnOnce(&mut Registry) -> Result<T, String>,
) -> Result<T, String> {
    REGISTRIES.with(|registries| {
        let mut registries = registries.borrow_mut();
        let registry = registries
            .get_mut(handle)
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("there is no open unit registry {handle}"))?;
        work(registry)
    })
}

fn text(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes).map_err(|_| "the text is not UTF-8".to_owned())
}

/// Keeps a registry open and gives its handle.
fn keep(registry: Registry) -> usize {
    REGISTRIES.with(|registries| {
        let mut registries = registries.borrow_mut();
        registries.push(Some(registry));
        registries.len() - 1
    })
}

/// Makes a registry of the builtin units and gives its handle.
pub fn open() -> usize {
    keep(Registry::new())
}

/// A copy of a registry with the units of a model, from the JSON `{"units": {...},
/// "unit_ids": {...}}` ([`Registry::with_model_units`]), whose handle is the answer as text.
///
/// # Errors
///
/// A request which is no such JSON and a handle of no registry.
pub fn with_model_units(handle: usize, request: &[u8]) -> Result<String, String> {
    let request: ModelUnitsRequest =
        serde_json::from_slice(request).map_err(|err| err.to_string())?;
    let copy = with_registry(handle, |registry| {
        Ok(registry.with_model_units(&request.units, &request.unit_ids))
    })?;
    Ok(keep(copy).to_string())
}

/// The unit expression of the entity of the buffer in the model of a registry, `{"unit":
/// "mmol/l"}`, `{"unit": null}` for an entity without one.
///
/// # Errors
///
/// A text which is not UTF-8 and a handle of no registry.
pub fn unit_of(handle: usize, entity: &[u8]) -> Result<String, String> {
    let entity = text(entity)?;
    with_registry(handle, |registry| {
        Ok(json!({"unit": registry.unit_of(entity)}).to_string())
    })
}

/// Defines a unit from the JSON `{"name": ..., "expression": ...}`; the answer is empty.
///
/// # Errors
///
/// A request which is no such JSON, a handle of no registry and the errors of
/// [`Registry::define`].
pub fn define(handle: usize, request: &[u8]) -> Result<String, String> {
    let request: DefineRequest = serde_json::from_slice(request).map_err(|err| err.to_string())?;
    with_registry(handle, |registry| {
        registry
            .define(&request.name, &request.expression)
            .map_err(|err| err.to_string())?;
        Ok(String::new())
    })
}

/// The factor of the expression of the buffer to the root units of pint and its dimensions,
/// `{"factor": f, "dimensions": {"[length]": 3}}`.
///
/// # Errors
///
/// A text which is not UTF-8, a handle of no registry and the errors of [`Registry::parse`].
pub fn parse(handle: usize, expression: &[u8]) -> Result<String, String> {
    let expression = text(expression)?;
    with_registry(handle, |registry| {
        let unit = registry.parse(expression).map_err(|err| err.to_string())?;
        let dimensions: serde_json::Map<String, Value> = unit
            .dimensions()
            .into_iter()
            .map(|(name, exponent)| (format!("[{name}]"), Value::from(exponent)))
            .collect();
        Ok(json!({"factor": number(unit.factor), "dimensions": dimensions}).to_string())
    })
}

/// The factor which converts a value between the units of the JSON `{"from": ..., "to":
/// ...}`, `{"factor": f}`.
///
/// # Errors
///
/// A request which is no such JSON, a handle of no registry and the errors of
/// [`Registry::factor`].
pub fn factor(handle: usize, request: &[u8]) -> Result<String, String> {
    let request: FactorRequest = serde_json::from_slice(request).map_err(|err| err.to_string())?;
    with_registry(handle, |registry| {
        let factor = registry
            .factor(&request.from, &request.to)
            .map_err(|err| err.to_string())?;
        Ok(json!({"factor": number(factor)}).to_string())
    })
}

/// Drops a registry; a handle of none is ignored.
pub fn close(handle: usize) {
    REGISTRIES.with(|registries| {
        if let Some(slot) = registries.borrow_mut().get_mut(handle) {
            *slot = None;
        }
    });
}

/// [`open`] of the C ABI.
#[unsafe(no_mangle)]
pub extern "C" fn units_open() -> usize {
    open()
}

/// [`define`] of the C ABI: the JSON at `pointer`, the answer or the message at the two words
/// of `out`, 1 for a unit defined and 0 for an error.
///
/// # Safety
///
/// `pointer` is a buffer of [`crate::allocate`] of `len` bytes, `out` two words of the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn units_define(
    handle: usize,
    pointer: *const u8,
    len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe { answer(define(handle, bytes(pointer, len)), out) }
}

/// [`parse`] of the C ABI, the expression at `pointer`, with the result of [`units_define`].
///
/// # Safety
///
/// As [`units_define`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn units_parse(
    handle: usize,
    pointer: *const u8,
    len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe { answer(parse(handle, bytes(pointer, len)), out) }
}

/// [`factor`] of the C ABI, the JSON at `pointer`, with the result of [`units_define`].
///
/// # Safety
///
/// As [`units_define`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn units_factor(
    handle: usize,
    pointer: *const u8,
    len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe { answer(factor(handle, bytes(pointer, len)), out) }
}

/// [`with_model_units`] of the C ABI, the JSON at `pointer`, with the result of
/// [`units_define`]: the answer is the handle of the copy as text.
///
/// # Safety
///
/// As [`units_define`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn units_with_model_units(
    handle: usize,
    pointer: *const u8,
    len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe { answer(with_model_units(handle, bytes(pointer, len)), out) }
}

/// [`unit_of`] of the C ABI, the entity at `pointer`, with the result of [`units_define`].
///
/// # Safety
///
/// As [`units_define`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn units_unit_of(
    handle: usize,
    pointer: *const u8,
    len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe { answer(unit_of(handle, bytes(pointer, len)), out) }
}

/// [`close`] of the C ABI.
#[unsafe(no_mangle)]
pub extern "C" fn units_close(handle: usize) {
    close(handle);
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{close, define, factor, open, parse, unit_of, with_model_units};

    fn read(answer: &str) -> Value {
        serde_json::from_str(answer).unwrap()
    }

    #[test]
    fn a_registry_through_the_interface() {
        let handle = open();
        assert_eq!(
            read(&parse(handle, b"mg/l").unwrap()),
            json!({"factor": 0.999_999_999_999_999_9, "dimensions": {"[length]": -3, "[mass]": 1}})
        );
        assert_eq!(
            read(&factor(handle, br#"{"from": "l", "to": "ml"}"#).unwrap()),
            json!({"factor": 1000.0})
        );
        define(
            handle,
            br#"{"name": "IU_insulin", "expression": "0.0347 * mg"}"#,
        )
        .unwrap();
        assert_eq!(
            read(&factor(handle, br#"{"from": "IU_insulin", "to": "ug"}"#).unwrap()),
            json!({"factor": 34.7})
        );
        let err = factor(handle, br#"{"from": "mg", "to": "ml"}"#).unwrap_err();
        assert_eq!(err, "cannot convert 'mg' ([mass]) to 'ml' ([length] ** 3)");
        assert!(
            parse(handle, b"degC")
                .unwrap_err()
                .contains("degree_Celsius")
        );
        close(handle);
        assert!(
            parse(handle, b"mg")
                .unwrap_err()
                .contains("no open unit registry")
        );
    }

    #[test]
    fn a_registry_with_the_units_of_a_model() {
        let handle = open();
        let model: usize = with_model_units(
            handle,
            br#"{"units": {"k": "mg_per_min"}, "unit_ids": {"mg_per_min": "mg/min"}}"#,
        )
        .unwrap()
        .parse()
        .unwrap();
        assert_ne!(model, handle);
        assert_eq!(
            read(&factor(model, br#"{"from": "mg_per_min/kg", "to": "mg/min/kg"}"#).unwrap()),
            json!({"factor": 1.0})
        );
        assert_eq!(
            read(&unit_of(model, b"k").unwrap()),
            json!({"unit": "mg_per_min"})
        );
        assert_eq!(read(&unit_of(model, b"x").unwrap()), json!({"unit": null}));
        // the registry it was copied from is as it was
        assert!(factor(handle, br#"{"from": "mg_per_min", "to": "mg/min"}"#).is_err());
        assert!(with_model_units(handle, br#"{"unknown": {}}"#).is_err());
        close(model);
        close(handle);
    }
}
