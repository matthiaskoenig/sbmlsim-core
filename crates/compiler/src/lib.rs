//! The compiler: SBML to `model.wasm` and its [`ModelDescription`].
//!
//! [`compile`] reads the SBML ([`sbmlsim_sbml::read`]) and builds the model representation
//! ([`sbmlsim_ir::build`]), so a refusal of the reader or of the builder surfaces with its own
//! message. The generation of the code is not there yet: it ends with
//! [`CompileError::Unsupported`] for `code generation`.
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
    /// The source of the Jacobian of the model.
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
    let _ = options;
    let doc = sbmlsim_sbml::read(sbml).map_err(from_read_error)?;
    let _model = sbmlsim_ir::build(&doc)?;
    Err(CompileError::Unsupported {
        feature: "code generation".to_string(),
        element: "model".to_string(),
    })
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
    fn a_readable_model_ends_at_the_code_generation() {
        let error = compile(DECAY, &CompileOptions::default()).unwrap_err();
        assert_eq!(
            error,
            CompileError::Unsupported {
                feature: "code generation".to_string(),
                element: "model".to_string()
            }
        );
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
