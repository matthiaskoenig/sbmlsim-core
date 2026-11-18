//! A mismatched model description must be rejected before it can select the wrong history ABI.

use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::Runtime;

#[test]
fn rejects_changed_history_site_count_or_version_before_instantiation() {
    let xml = include_str!("decay.xml");
    let delayed = xml.replace(
        "<ci>y</ci>",
        r#"<apply><csymbol definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><ci>y</ci><cn>1</cn></apply>"#,
    );
    let runtime = Runtime::embedded().unwrap();
    let mut ode = compile(xml, &CompileOptions::default()).unwrap();
    let mut dde = compile(&delayed, &CompileOptions::default()).unwrap();
    assert_eq!(ode.description.n_delays, 0);
    assert_eq!(dde.description.n_delays, 1);
    runtime.load(&dde.wasm, &dde.description).unwrap();

    ode.description.n_delays = 1;
    for count in [0, 2] {
        dde.description.n_delays = count;
        let error = runtime.load(&dde.wasm, &dde.description).err().unwrap();
        assert!(error.to_string().contains("history sites"), "{error}");
    }
    let error = runtime.load(&ode.wasm, &ode.description).err().unwrap();
    assert!(error.to_string().contains("history sites"), "{error}");

    dde.description.n_delays = 1;
    let section = sbmlsim_abi::history::SECTION.as_bytes();
    let version = dde
        .wasm
        .windows(section.len())
        .position(|s| s == section)
        .unwrap()
        + section.len();
    dde.wasm[version..version + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let error = runtime.load(&dde.wasm, &dde.description).err().unwrap();
    assert!(error.to_string().contains("history version"), "{error}");
}

/// The compiler bounds how far back a model can read only when every lag is a constant.
#[test]
fn the_description_bounds_the_history_of_constant_lags_only() {
    let xml = include_str!("decay.xml");
    let delay = |lag: &str| {
        xml.replace(
            "<ci>y</ci>",
            &format!(
                r#"<apply><csymbol definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><apply><csymbol definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><ci>y</ci><cn>2</cn></apply>{lag}</apply>"#
            ),
        )
    };
    let bound = |body: &str| {
        compile(body, &CompileOptions::default())
            .unwrap()
            .description
            .max_delay
    };
    assert_eq!(bound(xml), None, "a model without a delay has no history");
    // nested constant lags add up
    assert_eq!(bound(&delay("<cn>3</cn>")), Some(5.0));
    // a lag of the state or of the time leaves the reach unbounded
    assert_eq!(bound(&delay("<ci>y</ci>")), None);
    assert_eq!(
        bound(&delay(
            r#"<csymbol definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#
        )),
        None
    );
}
