use sbmlsim_abi::{IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::Runtime;

fn main() -> anyhow::Result<()> {
    let sbml = std::fs::read_to_string("model.xml")?;
    let compiled = compile(&sbml, &CompileOptions::default())?;

    let runtime = Runtime::embedded()?;
    let model = runtime.load(&compiled.wasm, &compiled.description)?;
    let mut instance = model.instantiate()?;

    let request = SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end: 10.0,
        output: Output::Uniform { steps: 100 },
        selections: vec!["time".to_string(), "[S]".to_string()],
        settings: IntegratorSettings::default(),
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        seed: 0,
        sensitivities: vec![],
    };
    let result = instance.simulate(&request)?;

    println!("{}", result.columns.join(","));
    for row in result.values.chunks(result.columns.len()) {
        println!("{row:?}");
    }
    Ok(())
}
