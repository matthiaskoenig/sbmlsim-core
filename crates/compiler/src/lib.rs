//! The compiler: SBML to `model.wasm` and its [`ModelDescription`].
//!
//! [`compile`] reads the SBML ([`sbmlsim_sbml::read`]), builds the model representation
//! ([`sbmlsim_ir::build`]) and emits the module and its description ([`sbmlsim_codegen::emit`]),
//! so a refusal of the reader, of the builder or of the code generation surfaces with its own
//! message.
//!
//! The crate does no file system or OS access, so it builds for `wasm32-unknown-unknown`.

use sbmlsim_abi::{JacobianChoice, ModelDescription};
use sbmlsim_sbml::ReadError;

pub use sbmlsim_ir::CompileError;

/// What the compiler does besides the translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileOptions {
    /// Optimize the generated module.
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
    let (wasm, description) = sbmlsim_codegen::emit_with(&model, options.jacobian)?;
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

    #[test]
    fn the_defaults_optimize_and_choose_the_jacobian() {
        let options = CompileOptions::default();
        assert!(options.optimize);
        assert_eq!(options.jacobian, JacobianChoice::Auto);
    }
}
