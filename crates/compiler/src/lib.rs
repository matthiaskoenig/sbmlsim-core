//! The compiler: SBML to `model.wasm` and its [`ModelDescription`].
//!
//! [`compile`] reads the SBML ([`sbmlsim_sbml::read`]), builds the model representation
//! ([`sbmlsim_ir::build`]) and emits the module and its description
//! ([`sbmlsim_codegen::emit_optimized`], [`sbmlsim_codegen::emit_with`] without the optimizer),
//! so a refusal of the reader, of the builder or of the code generation surfaces with its own
//! message.
//!
//! The crate does no file system or OS access, so it builds for `wasm32-unknown-unknown`.

use sbmlsim_abi::{JacobianChoice, ModelDescription};
use sbmlsim_sbml::ReadError;

pub use sbmlsim_codegen::Limits;
pub use sbmlsim_ir::CompileError;

/// What the compiler does besides the translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            optimize: true,
            jacobian: JacobianChoice::Auto,
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
    let model = sbmlsim_ir::build(&doc)?;
    let (wasm, description) = if options.optimize {
        sbmlsim_codegen::emit_optimized(&model, options.jacobian)?
    } else {
        sbmlsim_codegen::emit_with(&model, options.jacobian)?
    };
    Ok(CompiledModel { wasm, description })
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
    let model = sbmlsim_ir::build(&doc)?;
    let (wasm, description) =
        sbmlsim_codegen::emit_with_limits(&model, options.jacobian, options.optimize, limits)?;
    Ok(CompiledModel { wasm, description })
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
        assert_eq!(error.to_string(), "algebraic rule is not supported (A)");
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
    fn the_defaults_optimize_and_choose_the_jacobian() {
        let options = CompileOptions::default();
        assert!(options.optimize);
        assert_eq!(options.jacobian, JacobianChoice::Auto);
    }
}
