//! User-facing export commands use the same standalone source generator as the bindings.
use std::{path::Path, process::Command};

#[test]
fn command_line_exports_match_the_shared_generator() {
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/ode/events.xml");
    for format in [
        "python", "diffrax", "julia", "r", "markdown", "latex", "typst",
    ] {
        let expected = sbmlsim_ode::OdeSystem::read_path(&model, 10000)
            .unwrap()
            .render(
                format,
                &sbmlsim_ode::RenderOptions {
                    simulator: false,
                    standalone: false,
                    source: Some("events.xml".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_sbmlsim-core"))
            .args([
                "export",
                model.to_str().unwrap(),
                "--format",
                format,
                "--no-simulator",
                "--fragment",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected.content);
    }
}
