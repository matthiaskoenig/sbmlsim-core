//! The statistics of `compiler.wasm` natively (`stats_call`): the protocol of the requests and
//! the answers, their doubles as bits and their errors with the kind of the crate. That every
//! function gives the bits of python is held by `js/test/stats.test.mjs` on the wasm build.

#![cfg(feature = "stats")]

use sbmlsim_wasm::stats::{Answer, Failure, call};
use sbmlsim_wasm::{allocate, deallocate};
use serde_json::{Value, json};

/// The little-endian bytes of doubles.
fn bytes(values: &[f64]) -> Vec<u8> {
    values.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// The answer of a request of the values, its JSON parsed.
fn answer(request: &Value, values: &[f64]) -> (Value, Vec<f64>) {
    let Answer { json, values } =
        call(request.to_string().as_bytes(), &bytes(values)).expect("the call answers");
    (serde_json::from_str(&json).expect("JSON"), values)
}

/// The doubles of an array `{at, shape}` of an answer.
fn doubles<'a>(array: &Value, values: &'a [f64]) -> (&'a [f64], Vec<usize>) {
    let at = array["at"].as_u64().unwrap() as usize;
    let shape: Vec<usize> = array["shape"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as usize)
        .collect();
    let len: usize = shape.iter().product();
    (&values[at..at + len], shape)
}

#[test]
fn a_function_gives_its_doubles_as_bits() {
    let request = json!({
        "function": "quantile_ci",
        "x": {"at": 0, "shape": [5]},
        "q": {"at": 5, "shape": []},
    });
    let (json, values) = answer(&request, &[3.0, 1.0, f64::NAN, 2.0, 4.0, 0.5]);
    let tuple = json.as_array().unwrap();
    assert_eq!(tuple.len(), 4);
    assert_eq!(doubles(&tuple[0], &values).0, &[2.5]);
    assert_eq!(tuple[3], json!(4));
    let native =
        sbmlsim_stats::uncertainty::quantile_ci(&[3.0, 1.0, f64::NAN, 2.0, 4.0], 0.5, 0.95)
            .unwrap();
    for (k, expected) in [native.0, native.1, native.2].into_iter().enumerate() {
        assert_eq!(
            doubles(&tuple[k], &values).0[0].to_bits(),
            expected.to_bits()
        );
    }
}

#[test]
fn negative_zero_and_not_a_number_cross_as_bits() {
    let request = json!({"function": "rmse_from_mse", "mse": {"at": 0, "shape": []}});
    let (json, values) = answer(&request, &[-0.0]);
    assert_eq!(doubles(&json, &values).0[0].to_bits(), (-0.0_f64).to_bits());
    let request = json!({"function": "nrmse", "nres": {"at": 0, "shape": [1]}});
    let (json, values) = answer(&request, &[f64::NAN]);
    assert!(doubles(&json, &values).0[0].is_nan());
}

#[test]
fn a_matrix_keeps_its_shape_and_a_dictionary_its_keys() {
    let request = json!({
        "function": "fisher_statistics",
        "information": {"at": 0, "shape": [2, 2]},
        "cost": {"at": 4, "shape": []},
        "n": "20",
    });
    let (json, values) = answer(&request, &[2.0, 1.0, 1.0, 5.0, 0.05]);
    assert_eq!(json["rank"], json!(2));
    let (covariance, shape) = doubles(&json["covariance"], &values);
    assert_eq!(shape, [2, 2]);
    let native =
        sbmlsim_stats::fisher::statistics(&[2.0, 1.0, 1.0, 5.0], 2, 0.05, 20, false, 1e-8).unwrap();
    assert_eq!(covariance, native.covariance.as_slice());
    assert_eq!(doubles(&json["standard_errors"], &values).1, [2]);
}

#[test]
fn the_defaults_are_those_of_python() {
    let points = |extra: Value| {
        let mut request = json!({"function": "unit_points", "d": "2", "n": "4", "seed": "9"});
        request
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (json, values) = answer(&request, &[]);
        let (points, shape) = doubles(&json, &values);
        assert_eq!(shape, [4, 2]);
        points.to_vec()
    };
    let sobol = points(json!({}));
    assert_eq!(sobol, points(json!({"design": "sobol", "start": "0"})));
    assert_eq!(sobol, points(json!({"design": null})));
    let native = sbmlsim_stats::draws::unit_points(2, 4, sbmlsim_stats::draws::Design::Sobol, 9, 0);
    assert_eq!(sobol, native.unwrap());
    // a seed of 64 bits crosses exactly
    let (json, _) = answer(
        &json!({"function": "unit_points", "d": "1", "n": "1", "seed": u64::MAX.to_string(), "design": "random"}),
        &[],
    );
    assert_eq!(json["shape"], json!([1, 1]));
}

#[test]
fn an_error_has_the_kind_of_the_crate_and_the_message_of_python() {
    let failure = |request: Value, values: &[f64]| {
        call(request.to_string().as_bytes(), &bytes(values)).expect_err("the call fails")
    };
    let mse = failure(
        json!({"function": "mse", "residuals": {"at": 0, "shape": [0]}}),
        &[],
    );
    assert_eq!(
        mse,
        Failure {
            kind: "ValueError",
            message: "MSE requires at least one residual.".to_owned()
        }
    );
    let overflow = failure(
        json!({"function": "sd_se", "x": {"at": 0, "shape": [3]}}),
        &[0.0, 1e100, -1e100],
    );
    assert_eq!(overflow.kind, "OverflowError");
    let seed = failure(
        json!({"function": "unit_points", "d": "1", "n": "1", "seed": "-1"}),
        &[],
    );
    assert_eq!(
        (seed.kind, seed.message.as_str()),
        ("ValueError", "expected non-negative integer")
    );
    let scale = failure(
        json!({"function": "log_sd", "scale": "it's", "sd": {"at": 0, "shape": []}}),
        &[0.3],
    );
    assert!(
        scale
            .message
            .starts_with(r#"The parameter scale "it's" is none"#),
        "{}",
        scale.message
    );
}

#[test]
fn a_request_the_binding_cannot_read_is_a_type_error() {
    for (request, values, message) in [
        (json!({"function": "nope"}), vec![], "has no function"),
        (
            json!({"function": "sse"}),
            vec![],
            "missing field `residuals`",
        ),
        (
            json!({"function": "sse", "residuals": {"at": 0, "shape": [1]}, "extra": 1}),
            vec![1.0],
            "unknown field `extra`",
        ),
        (
            json!({"function": "sse", "residuals": {"at": 1, "shape": [2]}}),
            vec![1.0, 2.0],
            "past the 2 values",
        ),
        (
            json!({"function": "rmse_from_mse", "mse": {"at": 0, "shape": [2]}}),
            vec![1.0, 2.0],
            "a number is a double of the shape []",
        ),
        (
            json!({"function": "aic_from_mse", "mse": {"at": 0, "shape": []}, "n": 3, "k": "1"}),
            vec![1.0],
            "invalid type",
        ),
        (
            json!({"function": "family_level", "level": {"at": 0, "shape": []}, "m": "9223372036854775808"}),
            vec![0.95],
            "out of range",
        ),
        (json!([]), vec![], "not an object"),
    ] {
        let failure = call(request.to_string().as_bytes(), &bytes(&values)).expect_err("refused");
        assert_eq!(failure.kind, "TypeError", "{request}");
        assert!(
            failure.message.contains(message),
            "{request}: {}",
            failure.message
        );
    }
    let odd = call(br#"{"function": "constants"}"#, &[0; 3]).expect_err("refused");
    assert!(
        odd.message.contains("not a multiple of 8"),
        "{}",
        odd.message
    );
}

#[test]
fn the_constants() {
    let (json, values) = answer(&json!({"function": "constants"}), &[]);
    assert_eq!(json["BISECTIONS"], json!(52));
    assert_eq!(json["MAX_NEEDED"], json!(1 << 26));
    assert_eq!(doubles(&json["EPS"], &values).0, &[1e-12]);
    assert_eq!(json["MIXTURE"].as_array().unwrap().len(), 2);
}

/// Calls `stats_call` of the C ABI: the status and the two buffers it gives.
fn c_call(request: &[u8], values: &[u8]) -> (u32, Vec<u8>, Vec<u8>) {
    // SAFETY: the buffers come from `allocate` and are freed with their lengths
    unsafe {
        let put = |bytes: &[u8]| {
            let pointer = allocate(bytes.len());
            if !bytes.is_empty() {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
            }
            pointer
        };
        let (request_pointer, values_pointer) = (put(request), put(values));
        let out = allocate(4 * size_of::<usize>()).cast::<usize>();
        let status = sbmlsim_wasm::stats::stats_call(
            request_pointer,
            request.len(),
            values_pointer,
            values.len(),
            out,
        );
        let take = |k: usize| {
            let (pointer, len) = (*out.add(k) as *mut u8, *out.add(k + 1));
            let bytes = if len == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(pointer, len).to_vec()
            };
            deallocate(pointer, len);
            bytes
        };
        let (answer, doubles) = (take(0), take(2));
        deallocate(out.cast(), 4 * size_of::<usize>());
        deallocate(request_pointer, request.len());
        deallocate(values_pointer, values.len());
        (status, answer, doubles)
    }
}

#[test]
fn the_c_abi_writes_the_answer_or_the_error() {
    let request = br#"{"function": "sse", "residuals": {"at": 0, "shape": [2]}}"#;
    let (status, answer, values) = c_call(request, &bytes(&[3.0, 4.0]));
    assert_eq!(status, 1);
    assert_eq!(answer, br#"{"at":0,"shape":[]}"#);
    assert_eq!(values, 25.0_f64.to_le_bytes());
    let request = br#"{"function": "mse", "residuals": {"at": 0, "shape": [0]}}"#;
    let (status, answer, values) = c_call(request, &[]);
    assert_eq!(status, 0);
    let error: Value = serde_json::from_slice(&answer).unwrap();
    assert_eq!(
        error,
        json!({"kind": "ValueError", "message": "MSE requires at least one residual."})
    );
    assert!(values.is_empty());
}
