//! A PEtab problem through the interface of a page: opened, compiled, simulated by a host and
//! evaluated, against the evaluation of `sbmlsim-petab` on the native engine.
#![cfg(feature = "petab")]

use sbmlsim_abi::{ModelDescription, SimulationRequest};
use sbmlsim_host::Runtime;
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
fn a_page_evaluates_a_problem_as_the_native_executor_does() {
    let request = json!({ "yaml": "problem.yaml", "files": files() });
    let summary: Value =
        serde_json::from_str(&sbmlsim_wasm::petab::open(request.to_string().as_bytes()).unwrap())
            .unwrap();
    let handle = summary["handle"].as_u64().unwrap() as usize;
    assert_eq!(summary["parameters"], json!(["k1", "k2", "scale"]));

    // the host: compiles the model of the plan, simulates the requests
    let compiled = sbmlsim_wasm::petab::model(handle, 0).unwrap();
    let description: ModelDescription = serde_json::from_str(&compiled.description).unwrap();
    let runtime = Runtime::embedded().unwrap();
    let model = runtime.load(&compiled.wasm, &description).unwrap();
    let x = json!([0.5, 0.4, 3.0]);
    let requests: Vec<Value> = serde_json::from_str(
        &sbmlsim_wasm::petab::requests(handle, x.to_string().as_bytes()).unwrap(),
    )
    .unwrap();
    let results: Vec<Value> = requests
        .iter()
        .map(|planned| {
            let request: SimulationRequest =
                serde_json::from_value(planned["request"].clone()).unwrap();
            let result = model.instantiate().unwrap().simulate(&request).unwrap();
            serde_json::to_value(result).unwrap()
        })
        .collect();
    let input = json!({ "x": x, "results": results });
    let evaluation: Value = serde_json::from_str(
        &sbmlsim_wasm::petab::evaluate(handle, input.to_string().as_bytes()).unwrap(),
    )
    .unwrap();
    sbmlsim_wasm::petab::close(handle);
    assert!(sbmlsim_wasm::petab::requests(handle, b"").is_err());

    // the native executor of the same files
    let files: sbmlsim_petab::InMemory = files()
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, text)| (name.clone(), text.as_str().unwrap().as_bytes().to_vec()))
        .collect();
    let problem = sbmlsim_petab::PetabProblem::read("problem.yaml", &files).unwrap();
    let plan = sbmlsim_petab::Plan::new(&problem, &sbmlsim_petab::Options::default()).unwrap();
    let native = sbmlsim_petab::Executor::new(plan)
        .unwrap()
        .evaluate(&[0.5, 0.4, 3.0]);
    assert_eq!(evaluation["llh"].as_f64().unwrap(), native.llh);
    assert_eq!(evaluation["chi2"].as_f64().unwrap(), native.chi2);
    let simulated: Vec<f64> = serde_json::from_value(evaluation["simulated"].clone()).unwrap();
    assert_eq!(simulated, native.simulated);
    assert!(
        evaluation["simulation_table"]
            .as_str()
            .unwrap()
            .starts_with("observableId")
    );
}

#[test]
fn a_page_is_told_what_is_wrong_with_a_problem() {
    let mut files = files();
    files["observables.tsv"] = json!("observableId\tobservableFormula\nobs_a\tA\n");
    let request = json!({ "yaml": "problem.yaml", "files": files });
    let error = sbmlsim_wasm::petab::open(request.to_string().as_bytes()).unwrap_err();
    assert!(error.contains("noiseFormula"), "{error}");
}
