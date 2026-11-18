//! Algebraic rules through the compiled model and both host runtimes.

use std::sync::OnceLock;

use sbmlsim_abi::{
    Change, EngineError, IntegratorSettings, JacobianChoice, LinearSolver, Output,
    SIMULATION_CONTRACT, SimulationRequest, SimulationResult, SteadyState, Tolerance,
};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_engine::STEADY_STATE_FLAG;
use sbmlsim_host::{LoadedModel, Runtime};

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;
const TIME: &str =
    r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;

fn load(body: &str, options: CompileOptions) -> LoadedModel {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| Runtime::embedded().unwrap());
    let sbml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
<model id="dae">{body}</model>
</sbml>"#
    );
    let compiled = compile(&sbml, &options).unwrap();
    runtime.load(&compiled.wasm, &compiled.description).unwrap()
}

fn request(end: f64, output: Output, selections: &[&str]) -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end,
        output,
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        selections: selections.iter().map(|s| s.to_string()).collect(),
        settings: IntegratorSettings {
            relative_tolerance: 1e-10,
            absolute_tolerance: Tolerance::Scalar(1e-12),
            ..IntegratorSettings::default()
        },
        seed: 0,
    }
}

fn close(actual: f64, expected: f64) {
    let tolerance = 2e-8 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} within {tolerance}"
    );
}

fn same_bits(actual: &SimulationResult, expected: &SimulationResult) {
    assert_eq!(actual.columns, expected.columns);
    assert_eq!(actual.n_rows, expected.n_rows);
    assert_eq!(
        actual
            .values
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>(),
        expected
            .values
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>()
    );
}

fn decay(options: CompileOptions) -> LoadedModel {
    load(
        &format!(
            r#"<listOfParameters>
<parameter id="x" value="1" constant="false"/>
<parameter id="z" constant="false"/>
<parameter id="k" value="0.5" constant="true"/>
</listOfParameters>
<listOfRules>
<rateRule variable="x"><math {MATH}><apply><minus/><apply><times/><ci>k</ci><ci>x</ci></apply></apply></math></rateRule>
<algebraicRule><math {MATH}><apply><minus/><ci>z</ci><apply><times/><ci>x</ci><ci>x</ci></apply></apply></math></algebraicRule>
</listOfRules>"#
        ),
        options,
    )
}

fn assert_decay(result: &SimulationResult, k: f64) {
    assert_eq!(result.columns, ["time", "x", "z"]);
    assert!(result.n_rows >= 2);
    assert_eq!(result.values.len(), 3 * result.n_rows);
    close(result.values[0], 0.0);
    close(result.values[result.values.len() - 3], 2.0);
    for row in result.values.as_chunks::<3>().0 {
        let expected = (-k * row[0]).exp();
        close(row[1], expected);
        close(row[2], expected * expected);
        close(row[2], row[1] * row[1]);
    }
}

#[test]
fn klu_handles_a_differential_equation_without_a_diagonal_derivative() {
    let body = format!(
        r#"<listOfParameters>
<parameter id="x" value="0" constant="false"/>
<parameter id="z" constant="false"/>
</listOfParameters><listOfRules>
<rateRule variable="x"><math {MATH}><ci>z</ci></math></rateRule>
<algebraicRule><math {MATH}>
<apply><minus/><ci>z</ci><apply><plus/><ci>x</ci><cn>1</cn></apply></apply>
</math></algebraicRule></listOfRules>"#
    );
    for jacobian in [JacobianChoice::Analytic, JacobianChoice::DifferenceQuotient] {
        let loaded = load(
            &body,
            CompileOptions {
                jacobian,
                ..CompileOptions::default()
            },
        );
        let mut req = request(1.0, Output::Uniform { steps: 8 }, &["x", "z"]);
        req.settings.linear_solver = LinearSolver::Klu;
        req.settings.jacobian = jacobian;
        let result = loaded.instantiate().unwrap().simulate(&req).unwrap();
        assert_eq!(result.columns, ["time", "x", "z"]);
        for row in result.values.as_chunks::<3>().0 {
            close(row[1], row[0].exp() - 1.0);
            close(row[2], row[0].exp());
        }
    }
}

#[test]
fn the_dae_trajectory_agrees_with_the_analytic_solution() {
    for optimize in [false, true] {
        for jacobian in [JacobianChoice::Analytic, JacobianChoice::DifferenceQuotient] {
            let loaded = decay(CompileOptions { optimize, jacobian });
            let mut wasm = loaded.instantiate().unwrap();
            #[cfg(feature = "native")]
            let mut native = loaded.instantiate_native().unwrap();
            for linear_solver in [LinearSolver::Dense, LinearSolver::Klu] {
                let mut sampled = request(2.0, Output::Uniform { steps: 8 }, &["time", "x", "z"]);
                sampled.settings.linear_solver = linear_solver;
                sampled.settings.jacobian = jacobian;
                let result = wasm.simulate(&sampled).unwrap();
                assert_eq!(result.n_rows, 9);
                assert_decay(&result, 0.5);
                assert!(wasm.stats().unwrap().jacobian_evals > 0);
                #[cfg(feature = "native")]
                assert_decay(&native.simulate(&sampled).unwrap(), 0.5);

                let steps = SimulationRequest {
                    output: Output::IntegratorSteps,
                    ..sampled
                };
                assert_decay(&wasm.simulate(&steps).unwrap(), 0.5);
                #[cfg(feature = "native")]
                assert_decay(&native.simulate(&steps).unwrap(), 0.5);
            }
        }
    }
}

#[test]
fn prepared_and_ordinary_dae_runs_repeat_their_bits() {
    let loaded = decay(CompileOptions::default());
    let template = request(2.0, Output::Uniform { steps: 8 }, &["time", "x", "z"]);
    let plan = loaded.prepare(&template, &["k".into()]).unwrap();
    let mut wasm = loaded.instantiate().unwrap();
    #[cfg(feature = "native")]
    let mut native = loaded.instantiate_native().unwrap();
    let mut first_wasm = None;
    #[cfg(feature = "native")]
    let mut first_native = None;
    for k in [0.25, 0.9, 0.25] {
        let concrete = SimulationRequest {
            preinit: vec![("k".into(), k)],
            ..template.clone()
        };
        let expected = wasm.simulate(&concrete).unwrap();
        assert_decay(&expected, k);
        let prepared = wasm.simulate_prepared(&plan, &[k]).unwrap();
        same_bits(&prepared, &expected);
        if k == 0.25 {
            same_bits(&prepared, first_wasm.get_or_insert(prepared.clone()));
        }
        #[cfg(feature = "native")]
        {
            let expected = native.simulate(&concrete).unwrap();
            assert_decay(&expected, k);
            let prepared = native.simulate_prepared(&plan, &[k]).unwrap();
            same_bits(&prepared, &expected);
            if k == 0.25 {
                same_bits(&prepared, first_native.get_or_insert(prepared.clone()));
            }
        }
    }
}

fn assert_initial_selections(model: &LoadedModel, expected: &[(&str, f64)]) {
    let values = model.initial_selections().unwrap();
    assert_eq!(values.len(), model.description().selections.len());
    for &(id, value) in expected {
        let index = model
            .description()
            .selections
            .iter()
            .position(|selection| selection.id == id)
            .unwrap();
        close(values[index], value);
    }
}

fn assert_single_row_at_zero(result: SimulationResult, expected: &[(&str, f64)]) {
    assert_eq!(result.n_rows, 1);
    let expected = [("time", 0.0)].into_iter().chain(expected.iter().copied());
    assert_eq!(
        result.columns,
        expected.clone().map(|(id, _)| id).collect::<Vec<_>>()
    );
    assert_eq!(result.values.len(), result.columns.len());
    for (value, (_, expected)) in result.values.into_iter().zip(expected) {
        close(value, expected);
    }
}

#[test]
fn the_initial_event_uses_the_consistent_algebraic_value() {
    let loaded = load(
        &format!(
            r#"<listOfParameters>
<parameter id="x" value="2" constant="false"/>
<parameter id="observed" value="0" constant="false"/>
</listOfParameters>
<listOfRules>
<algebraicRule><math {MATH}><apply><minus/><ci>x</ci><cn>3</cn></apply></math></algebraicRule>
</listOfRules>
<listOfEvents>
<event useValuesFromTriggerTime="true">
<trigger initialValue="false" persistent="true"><math {MATH}><apply><gt/><ci>x</ci><cn>2.5</cn></apply></math></trigger>
<listOfEventAssignments>
<eventAssignment variable="observed"><math {MATH}><ci>x</ci></math></eventAssignment>
</listOfEventAssignments>
</event>
</listOfEvents>"#
        ),
        CompileOptions::default(),
    );
    assert_initial_selections(&loaded, &[("x", 3.0), ("observed", 0.0)]);
    let req = request(0.0, Output::Times(vec![0.0]), &["x", "observed"]);
    let expected = [("x", 3.0), ("observed", 3.0)];
    let mut wasm = loaded.instantiate().unwrap();
    assert_single_row_at_zero(wasm.simulate(&req).unwrap(), &expected);
    #[cfg(feature = "native")]
    {
        let mut native = loaded.instantiate_native().unwrap();
        assert_single_row_at_zero(native.simulate(&req).unwrap(), &expected);
    }
}

#[test]
fn a_change_and_cascading_events_leave_each_output_on_the_constraint() {
    let loaded = load(
        &format!(
            r#"<listOfParameters>
<parameter id="x" value="1" constant="false"/>
<parameter id="z" constant="false"/>
<parameter id="observed" value="0" constant="false"/>
</listOfParameters>
<listOfRules>
<rateRule variable="x"><math {MATH}><cn>0</cn></math></rateRule>
<algebraicRule><math {MATH}><apply><minus/><ci>z</ci><apply><times/><ci>x</ci><ci>x</ci></apply></apply></math></algebraicRule>
</listOfRules>
<listOfEvents>
<event id="jump" useValuesFromTriggerTime="true">
<trigger initialValue="true" persistent="true"><math {MATH}><apply><gt/><ci>z</ci><cn>3</cn></apply></math></trigger>
<listOfEventAssignments>
<eventAssignment variable="x"><math {MATH}><cn>3</cn></math></eventAssignment>
</listOfEventAssignments>
</event>
<event id="observe" useValuesFromTriggerTime="true">
<trigger initialValue="true" persistent="true"><math {MATH}><apply><gt/><ci>z</ci><cn>8</cn></apply></math></trigger>
<listOfEventAssignments>
<eventAssignment variable="observed"><math {MATH}><ci>z</ci></math></eventAssignment>
</listOfEventAssignments>
</event>
</listOfEvents>"#
        ),
        CompileOptions::default(),
    );
    let mut req = request(
        1.0,
        Output::IntegratorSteps,
        &["time", "x", "z", "observed"],
    );
    req.changes.push(Change {
        time: 0.5,
        values: vec![("x".into(), 2.0)],
        increments: vec![],
    });
    let result = loaded.instantiate().unwrap().simulate(&req).unwrap();
    assert_eq!(result.columns, ["time", "x", "z", "observed"]);
    assert_eq!(result.values.len(), 4 * result.n_rows);
    let rows = result.values.as_chunks::<4>().0;
    assert!(rows.len() >= 3);
    for row in rows {
        close(row[2], row[1] * row[1]);
        if row[0] < 0.5 {
            close(row[1], 1.0);
            close(row[3], 0.0);
        } else if row[0] > 0.5 {
            close(row[1], 3.0);
            close(row[3], 9.0);
        }
    }
    close(result.values[result.values.len() - 4], 1.0);
}

#[test]
fn an_event_refreshes_the_inputs_of_an_optimized_constraint() {
    let body = format!(
        r#"<listOfParameters>
<parameter id="z" constant="false"/>
<parameter id="q" value="1" constant="false"/>
<parameter id="r" value="2" constant="true"/>
</listOfParameters>
<listOfRules>
<algebraicRule><math {MATH}><apply><minus/><ci>z</ci><apply><plus/><ci>q</ci><ci>r</ci></apply></apply></math></algebraicRule>
</listOfRules>
<listOfEvents>
<event useValuesFromTriggerTime="true">
<trigger initialValue="false" persistent="true"><math {MATH}><apply><geq/>{TIME}<cn>1</cn></apply></math></trigger>
<listOfEventAssignments>
<eventAssignment variable="q"><math {MATH}><cn>17</cn></math></eventAssignment>
</listOfEventAssignments>
</event>
</listOfEvents>"#
    );
    for optimize in [false, true] {
        let loaded = load(
            &body,
            CompileOptions {
                optimize,
                ..CompileOptions::default()
            },
        );
        let mut wasm = loaded.instantiate().unwrap();
        let mut req = request(1.0, Output::Uniform { steps: 1 }, &["time", "q", "z"]);
        for absolute_tolerance in [Tolerance::Scalar(1e-12), Tolerance::PerState(vec![1e-12])] {
            req.settings.absolute_tolerance = absolute_tolerance;
            let result = wasm.simulate(&req).unwrap();
            assert_eq!(result.columns, ["time", "q", "z"]);
            assert_eq!(result.n_rows, 2);
            assert_eq!(result.values.len(), 6);
            for (actual, expected) in result
                .values
                .into_iter()
                .zip([0.0, 1.0, 3.0, 1.0, 17.0, 19.0])
            {
                close(actual, expected);
            }
        }
    }
}

#[test]
fn initial_assignments_and_algebraic_rules_are_solved_together() {
    let loaded = load(
        &format!(
            r#"<listOfParameters><parameter id="x" value="0" constant="false"/><parameter id="z" constant="false"/></listOfParameters>
<listOfInitialAssignments>
<initialAssignment symbol="x"><math {MATH}><apply><plus/><ci>z</ci><cn>1</cn></apply></math></initialAssignment>
</listOfInitialAssignments>
<listOfRules>
<rateRule variable="x"><math {MATH}><cn>0</cn></math></rateRule>
<algebraicRule><math {MATH}><apply><minus/><ci>z</ci><apply><times/><cn>2</cn><ci>x</ci></apply></apply></math></algebraicRule>
</listOfRules>"#
        ),
        CompileOptions::default(),
    );
    let expected = [("x", -1.0), ("z", -2.0)];
    assert_initial_selections(&loaded, &expected);
    let req = request(0.0, Output::Times(vec![0.0]), &["x", "z"]);
    let result = loaded.instantiate().unwrap().simulate(&req).unwrap();
    assert_single_row_at_zero(result, &expected);
}

#[test]
fn initial_concentrations_use_the_algebraic_compartment_size() {
    let loaded = load(
        &format!(
            r#"<listOfCompartments><compartment id="V" size="2" constant="false"/></listOfCompartments>
<listOfSpecies>
<species id="S" compartment="V" initialConcentration="4" hasOnlySubstanceUnits="false" boundaryCondition="true" constant="false"/>
</listOfSpecies>
<listOfRules>
<algebraicRule><math {MATH}><apply><minus/><ci>V</ci><cn>3</cn></apply></math></algebraicRule>
</listOfRules>"#
        ),
        CompileOptions::default(),
    );
    let expected = [("V", 3.0), ("S", 12.0), ("[S]", 4.0)];
    assert_initial_selections(&loaded, &expected);
    let mut req = request(0.0, Output::Times(vec![0.0]), &["V", "S", "[S]"]);
    let mut wasm = loaded.instantiate().unwrap();
    assert_single_row_at_zero(wasm.simulate(&req).unwrap(), &expected);

    req.preinit = vec![("[S]".into(), 5.0)];
    assert_single_row_at_zero(
        wasm.simulate(&req).unwrap(),
        &[("V", 3.0), ("S", 15.0), ("[S]", 5.0)],
    );
}

#[test]
fn preinit_conversion_uses_the_first_initialization_pass() {
    let loaded = load(
        &format!(
            r#"<listOfCompartments><compartment id="V" size="1" constant="false"/></listOfCompartments>
<listOfSpecies>
<species id="A" compartment="V" initialAmount="2" hasOnlySubstanceUnits="true" boundaryCondition="true" constant="false"/>
</listOfSpecies>
<listOfParameters><parameter id="z" constant="false"/></listOfParameters>
<listOfInitialAssignments>
<initialAssignment symbol="V"><math {MATH}><apply><plus/><ci>z</ci><ci>A</ci></apply></math></initialAssignment>
</listOfInitialAssignments>
<listOfRules>
<algebraicRule><math {MATH}><apply><minus/><ci>z</ci><cn>1</cn></apply></math></algebraicRule>
</listOfRules>"#
        ),
        CompileOptions::default(),
    );
    let mut req = request(0.0, Output::Times(vec![0.0]), &["z", "V", "A"]);
    req.preinit = vec![("[A]".into(), 4.0)];
    // The first pass reads A = 2 and z = 1, so the conversion uses V = 3.
    // The second pass reads the fixed A = 12, giving V = 13.
    let result = loaded.instantiate().unwrap().simulate(&req).unwrap();
    assert_single_row_at_zero(result, &[("z", 1.0), ("V", 13.0), ("A", 12.0)]);
}

#[test]
fn a_smooth_algebraic_constraint_is_consistent_at_a_nonzero_start() {
    let loaded = load(
        &format!(
            r#"<listOfParameters><parameter id="z" constant="false"/></listOfParameters>
<listOfRules>
<algebraicRule><math {MATH}><apply><minus/><ci>z</ci><apply><plus/><apply><times/>{TIME}{TIME}</apply><cn>1</cn></apply></apply></math></algebraicRule>
</listOfRules>"#
        ),
        CompileOptions::default(),
    );
    let mut req = request(3.0, Output::Uniform { steps: 2 }, &["time", "z"]);
    req.start = 2.0;
    let mut wasm = loaded.instantiate().unwrap();
    let result = wasm.simulate(&req).unwrap();
    assert_eq!(result.columns, ["time", "z"]);
    assert_eq!(result.n_rows, 3);
    assert_eq!(result.values.len(), 2 * result.n_rows);
    for (row, time) in result.values.as_chunks::<2>().0.iter().zip([2.0, 2.5, 3.0]) {
        close(row[0], time);
        close(row[1], time * time + 1.0);
    }

    req.steady_state = Some(SteadyState {
        max_time: 2.0,
        ..SteadyState::default()
    });
    let error = wasm.simulate(&req).unwrap_err();
    assert!(
        matches!(
            error,
            EngineError::Integration {
                flag: STEADY_STATE_FLAG,
                ..
            }
        ),
        "{error:?}"
    );
}

fn discontinuous_constraint(with_differential_state: bool) -> LoadedModel {
    let x = if with_differential_state {
        r#"<parameter id="x" value="0" constant="false"/>"#
    } else {
        ""
    };
    let rate = if with_differential_state {
        format!(r#"<rateRule variable="x"><math {MATH}><ci>z</ci></math></rateRule>"#)
    } else {
        String::new()
    };
    load(
        &format!(
            r#"<listOfParameters><parameter id="z" constant="false"/>{x}</listOfParameters>
<listOfRules>{rate}
<algebraicRule><math {MATH}><apply><minus/><ci>z</ci>
<piecewise><piece><cn>1</cn><apply><lt/>{TIME}<cn>1</cn></apply></piece><otherwise><cn>2</cn></otherwise></piecewise>
</apply></math></algebraicRule>
</listOfRules>"#
        ),
        CompileOptions::default(),
    )
}

fn assert_jump_rows(result: &SimulationResult, sampled: bool) {
    let width = result.columns.len();
    assert_eq!(result.values.len(), width * result.n_rows);
    assert!(result.n_rows >= 3);
    close(result.values[0], 0.0);
    close(result.values[result.values.len() - width], 2.0);
    if sampled {
        assert_eq!(result.n_rows, 9);
    } else {
        assert!(
            result
                .values
                .chunks_exact(width)
                .any(|row| (row[0] - 1.0).abs() < 1e-10),
            "integrator steps missed the discontinuity"
        );
    }
    for (index, row) in result.values.chunks_exact(width).enumerate() {
        let [time, z, differential @ ..] = row else {
            unreachable!("the trace has columns for t and z")
        };
        if sampled {
            close(*time, index as f64 * 0.25);
        }
        if *time < 1.0 {
            close(*z, 1.0);
        } else if *time > 1.0 {
            close(*z, 2.0);
        } else {
            assert!(
                (z - 1.0).abs().min((z - 2.0).abs()) <= 2e-8,
                "invalid algebraic value {z} at the discontinuity"
            );
        }
        if let [x] = differential {
            close(*x, time + (time - 1.0).max(0.0));
        }
    }
}

#[test]
fn a_pure_algebraic_jump_is_consistent_at_output_times_and_steps() {
    let loaded = discontinuous_constraint(false);
    let mut wasm = loaded.instantiate().unwrap();
    for output in [Output::Uniform { steps: 8 }, Output::IntegratorSteps] {
        let sampled = matches!(output, Output::Uniform { .. });
        let req = request(2.0, output, &["time", "z"]);
        let result = wasm.simulate(&req).unwrap();
        assert_eq!(result.columns, ["time", "z"]);
        assert_jump_rows(&result, sampled);
    }
}

#[test]
fn a_differential_state_integrates_the_algebraic_jump() {
    let loaded = discontinuous_constraint(true);
    let mut wasm = loaded.instantiate().unwrap();
    for output in [Output::Uniform { steps: 8 }, Output::IntegratorSteps] {
        let sampled = matches!(output, Output::Uniform { .. });
        let req = request(2.0, output, &["time", "z", "x"]);
        let result = wasm.simulate(&req).unwrap();
        assert_eq!(result.columns, ["time", "z", "x"]);
        assert_jump_rows(&result, sampled);
    }
}

/// A request change which feeds an assignment rule that an algebraic rule reads must
/// project against the refreshed rule, as an executed event does.
#[test]
fn a_change_refreshes_the_assignment_rules_the_projection_reads() {
    let loaded = load(
        &format!(
            r#"<listOfParameters>
<parameter id="k" value="1" constant="false"/>
<parameter id="q" value="0" constant="false"/>
<parameter id="x" value="0" constant="false"/>
<parameter id="z" value="0" constant="false"/>
</listOfParameters>
<listOfRules>
<assignmentRule variable="q"><math {MATH}><apply><times/><cn>2</cn><ci>k</ci></apply></math></assignmentRule>
<rateRule variable="x"><math {MATH}><cn>1</cn></math></rateRule>
<algebraicRule><math {MATH}><apply><minus/><ci>z</ci><ci>q</ci></apply></math></algebraicRule>
</listOfRules>"#
        ),
        CompileOptions::default(),
    );
    let mut req = request(2.0, Output::IntegratorSteps, &["time", "k", "q", "z"]);
    req.changes = vec![Change {
        time: 1.0,
        values: vec![("k".to_string(), 5.0)],
        increments: vec![],
    }];
    let result = loaded.instantiate().unwrap().simulate(&req).unwrap();
    assert_eq!(result.columns, ["time", "k", "q", "z"]);
    for row in result.values.as_chunks::<4>().0 {
        let [time, k, q, z] = *row;
        println!("t={time} k={k} q={q} z={z}");
    }
    let last = *result.values.as_chunks::<4>().0.last().unwrap();
    close(last[2], 10.0);
    close(last[3], 10.0);
    // the first row after the change carries the refreshed rule and its projection
    let after = result
        .values
        .as_chunks::<4>()
        .0
        .iter()
        .find(|r| r[1] == 5.0)
        .expect("a row after the change");
    close(after[2], 10.0);
    close(after[3], 10.0);
}
