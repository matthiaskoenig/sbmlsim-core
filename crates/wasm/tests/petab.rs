//! A PEtab problem through the interface of a page: opened, compiled, its requests simulated by
//! a host (here a table of known values) and evaluated.
#![cfg(feature = "petab")]

use sbmlsim_abi::{ModelDescription, Output, SimulationRequest};
use serde_json::{Value, json};

const MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="conversion">
    <listOfCompartments><compartment id="c" size="1" constant="true"/></listOfCompartments>
    <listOfSpecies>
      <species id="A" compartment="c" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
      <species id="B" compartment="c" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
    </listOfSpecies>
    <listOfParameters>
      <parameter id="k1" value="0.8" constant="true"/>
      <parameter id="k2" value="0.6" constant="true"/>
    </listOfParameters>
    <listOfReactions>
      <reaction id="forward" reversible="false">
        <listOfReactants><speciesReference species="A" stoichiometry="1" constant="true"/></listOfReactants>
        <listOfProducts><speciesReference species="B" stoichiometry="1" constant="true"/></listOfProducts>
        <kineticLaw><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>k1</ci><ci>A</ci></apply></math></kineticLaw>
      </reaction>
      <reaction id="backward" reversible="false">
        <listOfReactants><speciesReference species="B" stoichiometry="1" constant="true"/></listOfReactants>
        <listOfProducts><speciesReference species="A" stoichiometry="1" constant="true"/></listOfProducts>
        <kineticLaw><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>k2</ci><ci>B</ci></apply></math></kineticLaw>
      </reaction>
    </listOfReactions>
  </model>
</sbml>"#;

fn files() -> Value {
    json!({
        "problem.yaml": "format_version: 2.0.0\nparameter_files: [parameters.tsv]\nmodel_files:\n  m: {location: model.xml, language: sbml}\ncondition_files: [conditions.tsv]\nexperiment_files: [experiments.tsv]\nobservable_files: [observables.tsv]\nmeasurement_files: [measurements.tsv]\n",
        "model.xml": MODEL,
        "conditions.tsv": "conditionId\ttargetId\ttargetValue\nc0\tA\ta0\ndose\tA\tA + 1\n",
        "experiments.tsv": "experimentId\ttime\tconditionId\ne1\t0\tc0\ne1\t5\tdose\n",
        "observables.tsv": "observableId\tobservableFormula\tnoiseFormula\nobs_a\tA\t0.5\nobs_b\tscale * B\t0.2\n",
        "measurements.tsv": "observableId\texperimentId\ttime\tmeasurement\nobs_a\te1\t1\t0.7\nobs_a\te1\t5\t1.4\nobs_b\te1\t10\t1.1\n",
        "parameters.tsv": "parameterId\tlowerBound\tupperBound\tnominalValue\testimate\nk1\t0.01\t10\t0.8\ttrue\nk2\t0.01\t10\t0.6\ttrue\nscale\t0.1\t10\t2\ttrue\na0\t\t\t1.5\tfalse\n",
    })
}

#[test]
fn a_page_evaluates_a_problem_from_the_results_of_its_host() {
    let request = json!({ "yaml": "problem.yaml", "files": files() });
    let summary: Value =
        serde_json::from_str(&sbmlsim_wasm::petab::open(request.to_string().as_bytes()).unwrap())
            .unwrap();
    let handle = summary["handle"].as_u64().unwrap() as usize;
    assert_eq!(summary["parameters"], json!(["k1", "k2", "scale"]));
    assert_eq!(summary["nominal"], json!([0.8, 0.6, 2.0]));

    // the model of the plan carries the columns of the observables its requests select
    let compiled = sbmlsim_wasm::petab::model(handle, 0).unwrap();
    assert!(compiled.wasm.starts_with(b"\0asm"));
    let description: ModelDescription = serde_json::from_str(&compiled.description).unwrap();
    let x = json!([0.5, 0.4, 3.0]);
    let requests: Vec<Value> = serde_json::from_str(
        &sbmlsim_wasm::petab::requests(handle, x.to_string().as_bytes()).unwrap(),
    )
    .unwrap();
    assert_eq!(requests.len(), 1);
    let request: SimulationRequest =
        serde_json::from_value(requests[0]["request"].clone()).unwrap();
    assert!(request.preinit.contains(&("k1".to_string(), 0.5)));
    assert!(request.preinit.contains(&("[A]".to_string(), 1.5)));
    assert_eq!(request.changes.len(), 1, "the dose at 5 is a change");
    for column in &request.selections[1..] {
        assert!(
            description.parameters.iter().any(|p| p.id == *column),
            "{column} is no parameter of the model"
        );
    }

    // a host which simulated: every column the value 1 + its position, at the output times,
    // with the two rows of the change at 5
    let Output::Times(times) = &request.output else {
        panic!("the output is the times of the measurements")
    };
    let mut rows: Vec<f64> = times.clone();
    rows.push(5.0);
    rows.sort_by(f64::total_cmp);
    let width = request.selections.len();
    let mut values = Vec::new();
    for time in &rows {
        values.push(*time);
        values.extend((1..width).map(|k| 1.0 + k as f64));
    }
    let shape = json!({ "columns": request.selections, "n_rows": rows.len() });
    let input = json!({ "x": x, "results": [shape] });
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let evaluation: Value = serde_json::from_str(
        &sbmlsim_wasm::petab::evaluate(handle, input.to_string().as_bytes(), &bytes).unwrap(),
    )
    .unwrap();
    // obs_a is the first column (2), obs_b the second (3), of the noises 0.5 and 0.2
    let simulated: Vec<f64> = serde_json::from_value(evaluation["simulated"].clone()).unwrap();
    assert_eq!(simulated, [2.0, 2.0, 3.0]);
    let normal = |m: f64, y: f64, s: f64| {
        -0.5 * ((m - y) / s).powi(2) - s.ln() - 0.5 * (2.0 * std::f64::consts::PI).ln()
    };
    let llh = normal(0.7, 2.0, 0.5) + normal(1.4, 2.0, 0.5) + normal(1.1, 3.0, 0.2);
    assert!((evaluation["llh"].as_f64().unwrap() - llh).abs() < 1e-12);
    assert!(
        evaluation["simulation_table"]
            .as_str()
            .unwrap()
            .starts_with("observableId")
    );

    // values which are not those of the tables are refused
    for wrong in [
        &bytes[..bytes.len() - 8],
        &[bytes.as_slice(), &[0; 8]].concat(),
        &bytes[1..],
    ] {
        let error =
            sbmlsim_wasm::petab::evaluate(handle, input.to_string().as_bytes(), wrong).unwrap_err();
        assert!(error.contains("values"), "{error}");
    }

    // a value which is no number crosses as one: its measurement is simulated as NaN
    let mut nan = values.clone();
    nan[1] = f64::NAN;
    let nan: Vec<u8> = nan.iter().flat_map(|v| v.to_le_bytes()).collect();
    let evaluation: Value = serde_json::from_str(
        &sbmlsim_wasm::petab::evaluate(handle, input.to_string().as_bytes(), &nan).unwrap(),
    )
    .unwrap();
    assert_eq!(evaluation["simulated"][0], "NaN", "{evaluation}");
    assert_eq!(evaluation["llh"], "NaN", "{evaluation}");

    // a failed simulation is a log-likelihood of no number
    let input = json!({ "results": [{ "error": "the integration failed" }] });
    let evaluation: Value = serde_json::from_str(
        &sbmlsim_wasm::petab::evaluate(handle, input.to_string().as_bytes(), &[]).unwrap(),
    )
    .unwrap();
    assert_eq!(evaluation["llh"], "NaN", "{evaluation}");
    assert_eq!(
        evaluation["failures"],
        json!([[0, "the integration failed"]])
    );
    sbmlsim_wasm::petab::close(handle);
    assert!(sbmlsim_wasm::petab::requests(handle, b"").is_err());
}

#[test]
fn a_page_is_told_what_is_wrong_with_a_problem() {
    let mut files = files();
    files["observables.tsv"] = json!("observableId\tobservableFormula\nobs_a\tA\n");
    let request = json!({ "yaml": "problem.yaml", "files": files });
    let error = sbmlsim_wasm::petab::open(request.to_string().as_bytes()).unwrap_err();
    assert!(error.contains("noiseFormula"), "{error}");
}
