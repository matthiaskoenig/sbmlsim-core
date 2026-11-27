//! Actual complete generated files compared with their previous end-user outputs.
use sbmlsim_ode::{RenderOptions, export};

#[test]
fn repeated_reserved_symbols_do_not_change_collision_resolution() {
    let names = sbmlsim_ode::code::names(
        ["class", "class_", "class__", "class", "class_"],
        sbmlsim_ode::code::Target::Python,
    )
    .unwrap();
    assert_eq!(names.len(), 3);
    assert_eq!(names["class"], "class___");
    assert_eq!(names["class_"], "class_");
    assert_eq!(names["class__"], "class__");
}

#[test]
fn standalone_code_targets_preserve_complete_exports() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/ode");
    let mut failures = vec![];
    for label in ["demo", "repressilator", "events", "variable_compartment"] {
        let source = std::fs::read(root.join(format!("{label}.xml"))).unwrap();
        for fmt in [
            "python", "diffrax", "julia", "r", "markdown", "latex", "typst",
        ] {
            for simulator in [false, true] {
                let options = RenderOptions {
                    simulator,
                    standalone: simulator,
                    ..Default::default()
                };
                let actual = export(&source, fmt, &options);
                let reference =
                    std::fs::read_to_string(root.join(format!("{label}.{fmt}_{simulator}.txt")))
                        .unwrap()
                        .replace(
                            "sbmlode 0.3.3",
                            &format!("sbmlsim-core {}", env!("CARGO_PKG_VERSION")),
                        );
                match actual {
                    Err(error) => failures.push(
                        format!("{label}.{fmt}.{simulator}: {error}")
                            .chars()
                            .take(300)
                            .collect(),
                    ),
                    Ok(file) if file.content != reference => {
                        let lines = file
                            .content
                            .lines()
                            .zip(reference.lines())
                            .enumerate()
                            .filter(|(_, (a, b))| a != b)
                            .take(3)
                            .map(|(i, (a, b))| format!("line {} got {a:?}, expected {b:?}", i + 1))
                            .collect::<Vec<_>>()
                            .join("; ");
                        failures.push(format!(
                            "{label}.{fmt}.{simulator}: {lines} ({} vs {} bytes)",
                            file.content.len(),
                            reference.len()
                        ));
                    }
                    _ => {}
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} files differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
