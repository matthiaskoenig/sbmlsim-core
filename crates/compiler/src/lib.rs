//! The compiler: SBML to `model.wasm` and its [`ModelDescription`], or to a batched kernel and
//! its [`KernelDescription`].
//!
//! [`compile`] reads the SBML ([`sbmlsim_sbml::read`]), builds the model representation
//! ([`sbmlsim_ir::build`]), drives the species of [`CompileOptions::driven`]
//! ([`sbmlsim_ir::driven::apply`]) and emits the module and its description
//! ([`sbmlsim_codegen::emit_optimized`], [`sbmlsim_codegen::emit_with`] without the optimizer),
//! so a refusal of the reader, of the builder, of the driven species or of the code generation
//! surfaces with its own message. [`compile_kernel`] emits the batched kernel of the model instead (`kernel/1`,
//! [`sbmlsim_codegen::emit_kernel`]): the model with its RODAS4 integrator in one module, which
//! advances many points per call.
//!
//! The crate does no file system or OS access, so it builds for `wasm32-unknown-unknown`.

use sbmlsim_abi::{JacobianChoice, KernelDescription, ModelDescription};
use sbmlsim_sbml::ReadError;

pub use sbmlsim_codegen::{COMPILER, Limits, is_above_the_limits};
pub use sbmlsim_ir::CompileError;

/// What the compiler does besides the translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOptions {
    /// Optimize the generated module (`sbmlsim_codegen::emit_optimized`, the optimizer of
    /// `sbmlsim_ir::optimize`): constant folding and the rewrites which are exact in binary64,
    /// the assignment rules no output of a function reads left out, and the common
    /// subexpressions of a function computed once. The results are bitwise those of the module
    /// without it (`sbmlsim_codegen::emit_with`).
    pub optimize: bool,
    /// The source of the Jacobian of the model: `Auto` the analytic Jacobian where the model
    /// has one, else the colored difference quotient; `DifferenceQuotient` always the colored
    /// difference quotient; `Analytic` the analytic one, refused for a model which has none
    /// (see `sbmlsim_codegen::emit_with`).
    pub jacobian: JacobianChoice,
    /// Include the optional analytic parameter Jacobian when available. Disable this for
    /// ordinary simulations to avoid its derivative compilation and cached constants.
    /// Forward sensitivities still work through their numerical derivative fallback.
    pub parameter_jacobian: bool,
    /// The species to drive, SBML ids, in this order (see `sbmlsim_ir::driven`): each leaves
    /// the states and follows the path `X__value + X__slope * (time - driven__t_ref)` of
    /// three parameters, with the accumulator state `X__exchange` of the amount the model
    /// exchanged with it; the description lists them (`ModelDescription::driven`). Empty, the
    /// default, drives none.
    pub driven: Vec<String>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            optimize: true,
            jacobian: JacobianChoice::Auto,
            parameter_jacobian: true,
            driven: Vec::new(),
        }
    }
}

/// A compiled model.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledModel {
    /// The module `model.wasm`.
    pub wasm: Vec<u8>,
    /// The description of the model, `model.json`.
    pub description: ModelDescription,
}

/// Compiles the text of an SBML Level 3 Version 2 document.
pub fn compile(sbml: &str, options: &CompileOptions) -> Result<CompiledModel, CompileError> {
    let doc = sbmlsim_sbml::read(sbml).map_err(from_read_error)?;
    compile_document(&doc, options)
}

/// Compiles a document [`sbmlsim_sbml::read`] gave, e.g. one a caller added parameters and
/// rules to, as the PEtab problems of `sbmlsim-petab` add their observables to their models
/// (since 0.38.0).
pub fn compile_document(
    doc: &sbmlsim_sbml::Document,
    options: &CompileOptions,
) -> Result<CompiledModel, CompileError> {
    compile_limited(doc, options, Limits::default())
}

/// [`compile`] with other limits of the functions of `model.wasm` than the defaults
/// (`sbmlsim_codegen::MAX_FUNCTION_BYTES`, `MAX_LOCALS`), above which a function is split into
/// helper functions: lower ones split the functions of small models (the tests of the
/// splitting, which gives the bits of the module as one).
pub fn compile_with_limits(
    sbml: &str,
    options: &CompileOptions,
    limits: Limits,
) -> Result<CompiledModel, CompileError> {
    let doc = sbmlsim_sbml::read(sbml).map_err(from_read_error)?;
    compile_limited(&doc, options, limits)
}

/// The model representation of a document with the driven species of `options`, emitted
/// within `limits`.
fn compile_limited(
    doc: &sbmlsim_sbml::Document,
    options: &CompileOptions,
    limits: Limits,
) -> Result<CompiledModel, CompileError> {
    let model = sbmlsim_ir::driven::apply(sbmlsim_ir::build(doc)?, &options.driven)?;
    let (wasm, description) = sbmlsim_codegen::emit_with_options(
        &model,
        options.jacobian,
        options.optimize,
        options.parameter_jacobian,
        limits,
    )?;
    Ok(CompiledModel { wasm, description })
}

/// A compiled batched kernel (`kernel/1`).
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledKernel {
    /// The module `kernel.wasm`.
    pub wasm: Vec<u8>,
    /// The description of the kernel, `kernel.json`.
    pub description: KernelDescription,
}

/// Compiles the text of an SBML Level 3 Version 2 document into a batched kernel: the model with
/// its RODAS4 integrator in one module (`kernel/1`, see `sbmlsim_abi`). Refuses what the code
/// generation of the kernel refuses ([`sbmlsim_codegen::emit_kernel`]: events, delays,
/// algebraic rules, a model without states or without an analytic Jacobian) besides what
/// [`compile`] refuses.
pub fn compile_kernel(sbml: &str) -> Result<CompiledKernel, CompileError> {
    let doc = sbmlsim_sbml::read(sbml).map_err(from_read_error)?;
    let model = sbmlsim_ir::build(&doc)?;
    let (wasm, description) = sbmlsim_codegen::emit_kernel(&model)?;
    Ok(CompiledKernel { wasm, description })
}

/// A failure of the reader as a failure of the compiler.
fn from_read_error(error: ReadError) -> CompileError {
    match error {
        ReadError::Package(package) => CompileError::Unsupported {
            feature: "required package".to_string(),
            element: package,
        },
        other => CompileError::Invalid(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECAY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="m">
    <listOfParameters>
      <parameter id="k" value="1" constant="true"/>
    </listOfParameters>
  </model>
</sbml>"#;

    #[test]
    fn a_readable_model_compiles() {
        let compiled = compile(DECAY, &CompileOptions::default()).unwrap();
        assert!(compiled.wasm.starts_with(b"\0asm"));
        assert_eq!(compiled.description.id, "m");
        assert_eq!(compiled.description.parameters[0].id, "k");
        assert_eq!(compiled.description.parameters[0].initial, 1.0);
    }

    #[test]
    fn a_refusal_of_the_model_representation_has_its_message() {
        // (code generation refuses only functions above the limits of WebAssembly engines,
        // `sbmlsim-codegen`'s `a_function_above_the_limits_is_an_error`)
        let algebraic = DECAY.replace(
            "</model>",
            r#"<listOfRules><algebraicRule id="A"><math xmlns="http://www.w3.org/1998/Math/MathML"><ci>k</ci></math></algebraicRule></listOfRules></model>"#,
        );
        let error = compile(&algebraic, &CompileOptions::default()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "A determines no variable: every entity it reads is already determined (k is constant)"
        );
    }

    #[test]
    fn a_model_with_events_compiles() {
        let events = DECAY.replace(
            "</model>",
            r#"<listOfEvents><event id="E" useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math xmlns="http://www.w3.org/1998/Math/MathML"><true/></math></trigger></event></listOfEvents></model>"#,
        );
        let compiled = compile(&events, &CompileOptions::default()).unwrap();
        assert_eq!(compiled.description.events.len(), 1);
        assert_eq!(compiled.description.n_roots, 0);
    }

    #[test]
    fn a_refusal_of_the_reader_has_its_message() {
        let error = compile(
            "<sbml level=\"2\" version=\"4\"/>",
            &CompileOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(error, CompileError::Invalid(_)), "{error:?}");
        assert!(!error.to_string().is_empty());
    }

    /// The same SBML compiles to the same bytes of `model.wasm` and the same description, with
    /// the default limits and with limits which split the functions (no order of a hash map
    /// reaches the output).
    #[test]
    fn compiling_twice_gives_the_same_bytes() {
        let math = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;
        let model = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m">
<listOfParameters><parameter id="k" value="1" constant="true"/><parameter id="x" value="1" constant="false"/><parameter id="y" value="2" constant="false"/><parameter id="r" constant="false"/></listOfParameters>
<listOfRules><assignmentRule variable="r"><math {math}><apply><times/><ci>x</ci><ci>y</ci></apply></math></assignmentRule>
<rateRule variable="x"><math {math}><apply><minus/><apply><times/><ci>k</ci><ci>r</ci><apply><exp/><ci>y</ci></apply></apply></apply></math></rateRule>
<rateRule variable="y"><math {math}><apply><plus/><ci>r</ci><apply><exp/><ci>y</ci></apply></apply></math></rateRule></listOfRules>
<listOfEvents><event id="E" useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math {math}><apply><gt/><ci>x</ci><cn>3</cn></apply></math></trigger><listOfEventAssignments><eventAssignment variable="y"><math {math}><apply><times/><ci>r</ci><cn>2</cn></apply></math></eventAssignment></listOfEventAssignments></event></listOfEvents>
</model></sbml>"#
        );
        for optimize in [true, false] {
            let options = CompileOptions {
                optimize,
                ..CompileOptions::default()
            };
            let small = Limits {
                bytes: 60,
                locals: 7,
            };
            let a = compile(&model, &options).unwrap();
            let b = compile(&model, &options).unwrap();
            assert_eq!(a.wasm, b.wasm);
            assert_eq!(a.description, b.description);
            let c = compile_with_limits(&model, &options, small).unwrap();
            let d = compile_with_limits(&model, &options, small).unwrap();
            assert_eq!(c.wasm, d.wasm);
            assert_eq!(c.description, d.description);
            assert_ne!(a.wasm, c.wasm, "split");
        }
    }

    #[test]
    fn a_kernel_compiles_twice_to_the_same_bytes() {
        let math = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;
        let decay = DECAY.replace(
            "</model>",
            &format!(
                r#"<listOfRules><rateRule variable="y"><math {math}><apply><times/><apply><minus/><ci>k</ci></apply><ci>y</ci></apply></math></rateRule></listOfRules></model>"#
            ),
        )
        .replace(
            "</listOfParameters>",
            r#"<parameter id="y" value="1" constant="false"/></listOfParameters>"#,
        );
        let a = compile_kernel(&decay).unwrap();
        let b = compile_kernel(&decay).unwrap();
        assert_eq!(a, b);
        assert!(a.wasm.starts_with(b"\0asm"));
        assert_eq!(a.description.version, sbmlsim_abi::KERNEL_CONTRACT);
        assert_eq!(a.description.state_ids, ["y"]);
        assert_eq!(a.description.parameter_ids, ["k"]);
        // a model without states is refused with its feature
        let error = compile_kernel(DECAY).unwrap_err();
        assert!(error.to_string().contains("without states"), "{error}");
    }

    #[test]
    fn the_defaults_optimize_and_choose_the_jacobian() {
        let options = CompileOptions::default();
        assert!(options.optimize);
        assert_eq!(options.jacobian, JacobianChoice::Auto);
        assert!(options.driven.is_empty());
    }

    /// `X -> C` of the rate `k * X`, in units of the model.
    fn uptake() -> String {
        let math = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;
        format!(
            r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m" timeUnits="second" substanceUnits="mole"><listOfCompartments><compartment id="blood" size="2" units="litre" constant="true"/><compartment id="cell" size="1" units="litre" constant="true"/></listOfCompartments><listOfSpecies><species id="X" compartment="blood" initialConcentration="3" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/><species id="C" compartment="cell" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies><listOfParameters><parameter id="k" value="0.5" constant="true"/></listOfParameters><listOfReactions><reaction id="R" reversible="false"><listOfReactants><speciesReference species="X" stoichiometry="1" constant="true"/></listOfReactants><listOfProducts><speciesReference species="C" stoichiometry="1" constant="true"/></listOfProducts><kineticLaw><math {math}><apply><times/><ci>k</ci><ci>X</ci></apply></math></kineticLaw></reaction></listOfReactions></model></sbml>"#
        )
    }

    #[test]
    fn driven_species_compile() {
        for optimize in [true, false] {
            let options = CompileOptions {
                optimize,
                driven: vec!["X".to_string()],
                ..CompileOptions::default()
            };
            let compiled = compile(&uptake(), &options).unwrap();
            assert!(compiled.wasm.starts_with(b"\0asm"));
            let desc = &compiled.description;
            let states: Vec<&str> = desc.states.iter().map(|s| s.id.as_str()).collect();
            assert_eq!(states, ["C", "X__exchange"]);
            let index = |id: &str| desc.parameters.iter().position(|p| p.id == id).unwrap() as u32;
            assert_eq!(
                desc.driven,
                [sbmlsim_abi::DrivenSpecies {
                    id: "X".into(),
                    value: index("X__value"),
                    slope: index("X__slope"),
                    exchange: 1,
                    amount: false,
                }]
            );
            assert_eq!(desc.t_ref, Some(index("driven__t_ref")));
            assert_eq!(desc.parameters[index("X__value") as usize].initial, 3.0);
            // the new entities are selections, with their units
            for (id, unit) in [
                ("X__value", "mole/litre"),
                ("X__slope", "(mole/litre)/second"),
                ("driven__t_ref", "second"),
                ("X__exchange", "mole"),
            ] {
                assert!(desc.selections.iter().any(|s| s.id == id), "{id}");
                assert_eq!(desc.units[id], unit, "{id}");
            }
        }
        // the document entry point drives them too
        let doc = sbmlsim_sbml::read(&uptake()).unwrap();
        let options = CompileOptions {
            driven: vec!["X".to_string()],
            ..CompileOptions::default()
        };
        assert_eq!(
            compile_document(&doc, &options).unwrap(),
            compile(&uptake(), &options).unwrap()
        );
    }

    #[test]
    fn a_refusal_of_a_driven_species_has_its_message() {
        let options = CompileOptions {
            driven: vec!["k".to_string()],
            ..CompileOptions::default()
        };
        let error = compile(&uptake(), &options).unwrap_err();
        assert!(
            matches!(error, CompileError::Unsupported { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains('k'), "{error}");
    }
}
