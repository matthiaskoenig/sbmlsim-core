//! The CSV of a result of the command line (`sbmlsim-core simulate`) and of `js/simulate.mjs`.
//!
//! A header of the columns (ids of SBML, which hold no comma or quote) and a line per row; every
//! number is written in the shortest decimal which reads back as the same double (Rust's `{}` of
//! `f64`, which writes no exponent: `0.5`, `1`, `0.000001`, `-0`), and the values which are not
//! finite as `NaN`, `inf` and `-inf`. Two CSVs are equal byte for byte exactly when the results
//! are equal bitwise, every NaN equal to every NaN (the CSV writes no payload).

use std::fmt::Write as _;

use sbmlsim_abi::SimulationResult;

/// The CSV of a result.
pub fn to_csv(result: &SimulationResult) -> String {
    let mut csv = result.columns.join(",");
    csv.push('\n');
    let k = result.columns.len().max(1);
    for row in result.values.chunks(k) {
        for (j, value) in row.iter().enumerate() {
            if j > 0 {
                csv.push(',');
            }
            write!(csv, "{value}").unwrap();
        }
        csv.push('\n');
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_back_bitwise() {
        let values = vec![
            0.0,
            -0.0,
            1.0,
            0.1,
            1.0 / 3.0,
            1e-300,
            1.7976931348623157e308,
            5e-324,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let result = SimulationResult {
            sensitivities: None,
            local_errors: None,
            columns: vec!["time".to_string()],
            n_rows: values.len(),
            values: values.clone(),
        };
        let csv = to_csv(&result);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "time");
        assert_eq!(&lines[1..5], ["0", "-0", "1", "0.1"]);
        assert_eq!(&lines[9..], ["NaN", "inf", "-inf"]);
        for (line, value) in lines[1..].iter().zip(&values) {
            let read: f64 = line.parse().unwrap();
            assert!(read.to_bits() == value.to_bits() || (read.is_nan() && value.is_nan()));
        }
    }
}
