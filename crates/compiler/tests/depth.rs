//! The depth limit of a math: a math nested `MAX_MATH_DEPTH` elements deep compiles, one level
//! deeper is refused with `CompileError::Invalid`, also through inlined function definitions,
//! and a document nested far deeper is refused before the XML parser (which recurses per
//! element) sees it, instead of exhausting the stack.
//!
//! The models compile in a thread with a stack of 2 MiB (the default of a thread Rust spawns) in
//! an optimized build, of 64 MiB in a debug build, whose frames are larger. Natively (x86_64)
//! the inlining of nested calls of function definitions takes about 5 KiB of stack per level,
//! more than the build for wasm32, whose margin `MAX_MATH_DEPTH` documents.

use sbmlsim_compiler::{CompileError, CompileOptions, compile};
use sbmlsim_sbml::MAX_XML_DEPTH;
use sbmlsim_sbml::math::MAX_MATH_DEPTH;

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

/// A model of the state `x` whose rate rule is `body`, with the function definitions `functions`.
fn model(body: &str, functions: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="deep">
<listOfFunctionDefinitions>{functions}</listOfFunctionDefinitions>
<listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}>{body}</math></rateRule></listOfRules>
</model></sbml>"#
    )
}

/// `x + (x + (... + 1))`, `n` applies: the deepest elements (`<plus/>`, `<ci>`, `<cn>` of the
/// innermost apply) are at the depth `n + 1` below `<math>`.
fn sums(n: usize) -> String {
    model(
        &format!(
            "{}<cn>1</cn>{}",
            "<apply><plus/><ci>x</ci>".repeat(n),
            "</apply>".repeat(n)
        ),
        "",
    )
}

/// `sin(sin(... x))`, `n` applies: depth `n + 1`.
fn sines(n: usize) -> String {
    model(
        &format!(
            "{}<ci>x</ci>{}",
            "<apply><sin/>".repeat(n),
            "</apply>".repeat(n)
        ),
        "",
    )
}

/// `n` nested `piecewise(…, x > 0, 0)` (two elements per level, the condition of the innermost
/// one at the depth `2n + 2`).
fn piecewise(n: usize) -> String {
    model(
        &format!(
            "{}<ci>x</ci>{}",
            "<piecewise><piece>".repeat(n),
            "<apply><gt/><ci>x</ci><cn>0</cn></apply></piece><otherwise><cn>0</cn></otherwise></piecewise>"
                .repeat(n)
        ),
        "",
    )
}

/// `f(f(... x))` with `f(a) = (a + 1) * 2`, `n` calls: depth `n + 1` as written, `2n + 1`
/// with `f` inlined (two levels per call).
fn calls(n: usize) -> String {
    let f = format!(
        r#"<functionDefinition id="f"><math {MATH}><lambda><bvar><ci>a</ci></bvar><apply><times/><apply><plus/><ci>a</ci><cn>1</cn></apply><cn>2</cn></apply></lambda></math></functionDefinition>"#
    );
    model(
        &format!(
            "{}<ci>x</ci>{}",
            "<apply><ci>f</ci>".repeat(n),
            "</apply>".repeat(n)
        ),
        &f,
    )
}

/// Compiles in a thread with the stack of the module documentation.
fn compile_on_a_small_stack(sbml: String) -> Result<(), CompileError> {
    let stack = if cfg!(debug_assertions) {
        64 << 20
    } else {
        2 << 20
    };
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || compile(&sbml, &CompileOptions::default()).map(|_| ()))
        .unwrap()
        .join()
        .unwrap()
}

fn assert_too_deep(result: Result<(), CompileError>, what: &str) {
    match result {
        Err(CompileError::Invalid(message)) => {
            assert!(message.contains("nested deeper than"), "{what}: {message}")
        }
        other => panic!("{what}: {other:?}"),
    }
}

#[test]
fn a_math_at_the_limit_compiles_and_one_level_deeper_is_refused() {
    assert_eq!(MAX_MATH_DEPTH, 400);
    let n = MAX_MATH_DEPTH - 1;
    compile_on_a_small_stack(sums(n)).unwrap();
    assert_too_deep(compile_on_a_small_stack(sums(n + 1)), "sums");
    compile_on_a_small_stack(sines(n)).unwrap();
    assert_too_deep(compile_on_a_small_stack(sines(n + 1)), "sines");
    let n = MAX_MATH_DEPTH / 2 - 1;
    compile_on_a_small_stack(piecewise(n)).unwrap();
    assert_too_deep(compile_on_a_small_stack(piecewise(n + 1)), "piecewise");
}

#[test]
fn an_inlined_function_definition_counts_towards_the_limit() {
    // as written within the limit, inlined at it and one level deeper than it
    let n = MAX_MATH_DEPTH / 2 - 1;
    compile_on_a_small_stack(calls(n)).unwrap();
    let result = compile_on_a_small_stack(calls(n + 1));
    match result {
        Err(CompileError::Invalid(message)) => {
            assert!(
                message.contains("function definitions it calls inlined"),
                "{message}"
            )
        }
        other => panic!("{other:?}"),
    }
    // a chain of definitions, each within the limit, whose inlined body is far deeper: refused
    // when the inlining passes the limit, before a deeper expression is built
    let mut functions = String::new();
    let levels = 40;
    for k in 0..levels {
        let inner = if k + 1 == levels {
            "<ci>a</ci>".to_string()
        } else {
            format!("<apply><ci>g{}</ci><ci>a</ci></apply>", k + 1)
        };
        let body = format!(
            "{}{inner}{}",
            "<apply><plus/><cn>1</cn>".repeat(30),
            "</apply>".repeat(30)
        );
        functions.push_str(&format!(
            r#"<functionDefinition id="g{k}"><math {MATH}><lambda><bvar><ci>a</ci></bvar>{body}</lambda></math></functionDefinition>"#
        ));
    }
    let chain = model("<apply><ci>g0</ci><ci>x</ci></apply>", &functions);
    assert_too_deep(compile_on_a_small_stack(chain), "chain");
}

#[test]
fn a_document_nested_far_deeper_is_refused_before_it_is_parsed() {
    // 100000 levels exhausted the stack of the XML parser
    let n = 100_000;
    match compile_on_a_small_stack(sums(n)) {
        Err(CompileError::Invalid(message)) => assert!(
            message.contains(&format!("nested deeper than {MAX_XML_DEPTH} elements")),
            "{message}"
        ),
        other => panic!("{other:?}"),
    }
    // the scan of the nesting skips comments, CDATA, processing instructions and quoted `>`
    let deep_comment = format!("<!-- {} -->", "<a>".repeat(2 * MAX_XML_DEPTH));
    let xml = sums(10).replace(
        r#"<model id="deep">"#,
        &format!(
            r#"<model id="deep" name="a > b">{deep_comment}<?pi <a><a> ?><notes><![CDATA[{}]]></notes>"#,
            "<a>".repeat(2 * MAX_XML_DEPTH)
        ),
    );
    compile_on_a_small_stack(xml).unwrap();
}

#[test]
fn shallow_alias_chains_are_refused_before_exhausting_the_stack() {
    let functions = (0..10_000).map(|k| {
        let body = if k == 9_999 { "<cn>1</cn>".into() }
            else { format!("<apply><ci>f{}</ci></apply>", k + 1) };
        format!(r#"<functionDefinition id="f{k}"><math {MATH}><lambda>{body}</lambda></math></functionDefinition>"#)
    }).collect::<String>();
    // Use the default thread stack, unlike the intentionally very deep MathML tests.
    let xml = model("<apply><ci>f0</ci></apply>", &functions);
    let result = std::thread::spawn(move || compile(&xml, &CompileOptions::default()))
        .join()
        .unwrap();
    assert!(
        matches!(result, Err(CompileError::Invalid(ref message)) if message.contains("function expansion depth limit")),
        "{result:?}"
    );
}

#[test]
fn repeated_argument_substitution_has_a_construction_budget() {
    let functions = format!(
        r#"<functionDefinition id="dup"><math {MATH}><lambda><bvar><ci>a</ci></bvar><apply><plus/><ci>a</ci><ci>a</ci></apply></lambda></math></functionDefinition>"#
    );
    let body = format!(
        "{}<ci>x</ci>{}",
        "<apply><ci>dup</ci>".repeat(20),
        "</apply>".repeat(20)
    );
    let result = compile_on_a_small_stack(model(&body, &functions));
    assert!(
        matches!(result, Err(CompileError::Invalid(ref message)) if message.contains("expansion budget")),
        "{result:?}"
    );
}

#[test]
fn wide_xor_piecewise_compiles_with_linear_output_growth() {
    let compile_xor = |n| {
        let terms = (0..n)
            .map(|k| format!("<apply><gt/><ci>x</ci><cn>{k}</cn></apply>"))
            .collect::<String>();
        let body = format!(
            "<piecewise><piece><cn>1</cn><apply><xor/>{terms}</apply></piece><otherwise><cn>0</cn></otherwise></piecewise>"
        );
        let xml = model(&body, "");
        for optimize in [false, true] {
            let result = compile(
                &xml,
                &CompileOptions {
                    optimize,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(result.wasm.len() < 100_000);
        }
        compile(&xml, &CompileOptions::default())
            .unwrap()
            .wasm
            .len()
    };
    let small = compile_xor(32);
    let large = compile_xor(128);
    assert!(large < small * 5, "{small} -> {large}");
}

#[test]
fn recursive_function_definitions_return_a_compile_error() {
    let functions = format!(
        r#"<functionDefinition id="f"><math {MATH}><lambda><apply><ci>g</ci></apply></lambda></math></functionDefinition><functionDefinition id="g"><math {MATH}><lambda><apply><ci>f</ci></apply></lambda></math></functionDefinition>"#
    );
    let result = compile(
        &model("<apply><ci>f</ci></apply>", &functions),
        &CompileOptions::default(),
    );
    assert!(
        matches!(result, Err(CompileError::Invalid(ref message)) if message.contains("calls itself")),
        "{result:?}"
    );
}

#[test]
fn quadratic_piecewise_root_expansion_is_refused_before_construction() {
    let pieces = (0..500)
        .map(|k| format!("<piece><cn>{k}</cn><apply><gt/><ci>x</ci><cn>{k}</cn></apply></piece>"))
        .collect::<String>();
    let body = format!("<piecewise>{pieces}<otherwise><cn>0</cn></otherwise></piecewise>");
    let result = compile(&model(&body, ""), &CompileOptions::default());
    assert!(
        matches!(result, Err(CompileError::Invalid(ref message)) if message.contains("discontinuity root construction")),
        "{result:?}"
    );
}

#[test]
fn nested_boolean_piecewise_sign_expansion_is_bounded() {
    let mut condition = "<apply><gt/><ci>x</ci><cn>0</cn></apply>".to_string();
    for _ in 0..30 {
        condition = format!(
            "<piecewise><piece><true/>{condition}</piece><otherwise><false/></otherwise></piecewise>"
        );
    }
    let body = format!(
        "<piecewise><piece><cn>1</cn>{condition}</piece><otherwise><cn>0</cn></otherwise></piecewise>"
    );
    let result = compile(&model(&body, ""), &CompileOptions::default());
    assert!(
        matches!(result, Err(CompileError::Invalid(ref message)) if message.contains("discontinuity root construction")),
        "{result:?}"
    );
}
