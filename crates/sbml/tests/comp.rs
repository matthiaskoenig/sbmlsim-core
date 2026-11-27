//! The flattening of composed models (the package `comp`): the ids of the submodels, the
//! deletions, the replacements, the conversions and the external model definitions. The cases of
//! the package of the SBML Test Suite are simulated from their composed models by
//! `sbmlsim-core testsuite --comp`.

use sbmlsim_sbml::math::{Node, Op};
use sbmlsim_sbml::{Document, ReadError, Rule, flatten_comp, read, read_with};

/// A document of `comp` around a model and model definitions.
fn composed(model: &str, definitions: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:comp="http://www.sbml.org/sbml/level3/version1/comp/version1" level="3" version="2" comp:required="true">
{model}
<comp:listOfModelDefinitions>{definitions}</comp:listOfModelDefinitions>
</sbml>"#
    )
}

fn parameters(doc: &Document) -> Vec<(&str, Option<f64>)> {
    doc.model
        .parameters
        .iter()
        .map(|p| (p.id.as_str(), p.value))
        .collect()
}

fn symbol(id: &str) -> Node {
    Node::Symbol(id.into())
}

fn apply(op: Op, args: Vec<Node>) -> Node {
    Node::Apply { op, args }
}

/// The math of the assignment rule of a variable.
fn rule(doc: &Document, variable: &str) -> Node {
    doc.model
        .rules
        .iter()
        .find_map(|r| match r {
            Rule::Assignment {
                variable: v, math, ..
            } if v == variable => math.clone(),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no assignment rule of {variable}"))
}

const DECAY: &str = r#"<comp:modelDefinition id="decay">
  <listOfParameters>
    <parameter id="k" value="2" constant="true"/>
    <parameter id="x" value="1" constant="false"/>
    <parameter id="y" constant="false"/>
  </listOfParameters>
  <listOfRules>
    <rateRule variable="x"><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>k</ci><ci>x</ci></apply></math></rateRule>
    <assignmentRule variable="y"><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><plus/><ci>x</ci><csymbol definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol></apply></math></assignmentRule>
  </listOfRules>
</comp:modelDefinition>"#;

#[test]
fn a_composed_model_without_submodels_is_its_model() {
    // `Armistead_CellDeathDis2024` of the PEtab benchmark collection requires `comp` and has
    // no submodel; so does a composed model of pkdb_models with `distrib` beside it
    for required in [
        r#"comp:required="true""#,
        r#"comp:required="true" distrib:required="true""#,
        r#"distrib:required="true" comp:required="true""#,
    ] {
        let xml = format!(
            r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:comp="http://www.sbml.org/sbml/level3/version1/comp/version1" xmlns:distrib="http://www.sbml.org/sbml/level3/version1/distrib/version1" level="3" version="2" {required}><model id="m"><listOfParameters><parameter id="p" value="1" constant="true"/></listOfParameters></model></sbml>"#
        );
        let doc = read(&xml).unwrap();
        assert_eq!(doc.model.id.as_deref(), Some("m"), "{required}");
        assert_eq!(parameters(&doc), [("p", Some(1.0))]);
    }
}

#[test]
fn the_elements_of_a_submodel_get_its_id_as_prefix() {
    let xml = composed(
        r#"<model id="main">
  <listOfParameters><parameter id="k" value="5" constant="true"/></listOfParameters>
  <comp:listOfSubmodels>
    <comp:submodel comp:id="A" comp:modelRef="decay"/>
    <comp:submodel comp:id="B" comp:modelRef="outer"/>
  </comp:listOfSubmodels>
</model>"#,
        &format!(
            r#"{DECAY}<comp:modelDefinition id="outer">
  <comp:listOfSubmodels><comp:submodel comp:id="inner" comp:modelRef="decay"/></comp:listOfSubmodels>
</comp:modelDefinition>"#
        ),
    );
    let doc = read(&xml).unwrap();
    let ids: Vec<&str> = parameters(&doc).iter().map(|(id, _)| *id).collect();
    assert_eq!(
        ids,
        [
            "k",
            "A__k",
            "A__x",
            "A__y",
            "B__inner__k",
            "B__inner__x",
            "B__inner__y"
        ]
    );
    // the math reads the elements of its own submodel
    assert_eq!(
        doc.model.rules[0],
        Rule::Rate {
            id: None,
            variable: "A__x".into(),
            math: Some(apply(Op::Times, vec![symbol("A__k"), symbol("A__x")])),
        }
    );
    assert_eq!(
        rule(&doc, "B__inner__y"),
        apply(Op::Plus, vec![symbol("B__inner__x"), Node::CsymbolTime])
    );
}

#[test]
fn a_replaced_element_is_its_replacement_divided_by_the_conversion_factor() {
    // `x` of the submodel is `X / f`: the math which reads it reads `X / f`, and the rate
    // rule which assigns it assigns `f` times its math to `X`
    let xml = composed(
        r#"<model id="main">
  <listOfParameters>
    <parameter id="X" value="3" constant="false">
      <comp:listOfReplacedElements>
        <comp:replacedElement comp:submodelRef="A" comp:idRef="x" comp:conversionFactor="f"/>
      </comp:listOfReplacedElements>
    </parameter>
    <parameter id="f" value="10" constant="true"/>
  </listOfParameters>
  <comp:listOfSubmodels><comp:submodel comp:id="A" comp:modelRef="decay"/></comp:listOfSubmodels>
</model>"#,
        DECAY,
    );
    let doc = read(&xml).unwrap();
    let x_over_f = apply(Op::Divide, vec![symbol("X"), symbol("f")]);
    assert_eq!(
        parameters(&doc),
        [
            ("X", Some(3.0)),
            ("f", Some(10.0)),
            ("A__k", Some(2.0)),
            ("A__y", None)
        ]
    );
    assert_eq!(
        doc.model.rules[0],
        Rule::Rate {
            id: None,
            variable: "X".into(),
            math: Some(apply(
                Op::Times,
                vec![
                    apply(Op::Times, vec![symbol("A__k"), x_over_f.clone()]),
                    symbol("f")
                ]
            )),
        }
    );
    assert_eq!(
        rule(&doc, "A__y"),
        apply(Op::Plus, vec![x_over_f, Node::CsymbolTime])
    );
}

#[test]
fn a_replacement_takes_the_id_of_the_element_it_replaces() {
    // `K` of the model is replaced by `k` of the submodel, which becomes `K`; a reference by a
    // port and a deletion by a meta id
    let xml = composed(
        r#"<model id="main">
  <listOfParameters>
    <parameter id="K" value="7" constant="true">
      <comp:replacedBy comp:submodelRef="A" comp:portRef="k_port"/>
    </parameter>
  </listOfParameters>
  <comp:listOfSubmodels>
    <comp:submodel comp:id="A" comp:modelRef="ported">
      <comp:listOfDeletions><comp:deletion comp:metaIdRef="the_y"/></comp:listOfDeletions>
    </comp:submodel>
  </comp:listOfSubmodels>
</model>"#,
        r#"<comp:modelDefinition id="ported">
  <listOfParameters>
    <parameter id="k" value="2" constant="true"/>
    <parameter id="x" value="1" constant="false"/>
    <parameter metaid="the_y" id="y" value="0" constant="true"/>
  </listOfParameters>
  <listOfRules>
    <rateRule variable="x"><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>k</ci><ci>x</ci></apply></math></rateRule>
  </listOfRules>
  <comp:listOfPorts><comp:port comp:id="k_port" comp:idRef="k"/></comp:listOfPorts>
</comp:modelDefinition>"#,
    );
    let doc = read(&xml).unwrap();
    assert_eq!(parameters(&doc), [("K", Some(2.0)), ("A__x", Some(1.0))]);
    assert_eq!(
        doc.model.rules[0].math(),
        Some(&apply(Op::Times, vec![symbol("K"), symbol("A__x")]))
    );
}

#[test]
fn a_deleted_local_parameter_uncovers_the_parameter_of_the_model() {
    // case 01159 of the SBML Test Suite: the kinetic law reads the global `p1` once its local
    // `p1` is deleted
    let xml = composed(
        r#"<model id="main">
  <comp:listOfSubmodels>
    <comp:submodel comp:id="A" comp:modelRef="local">
      <comp:listOfDeletions><comp:deletion comp:metaIdRef="local_p1"/></comp:listOfDeletions>
    </comp:submodel>
  </comp:listOfSubmodels>
</model>"#,
        r#"<comp:modelDefinition id="local">
  <listOfCompartments><compartment id="C" size="1" constant="true"/></listOfCompartments>
  <listOfSpecies><species id="s" compartment="C" initialAmount="1" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="false"/></listOfSpecies>
  <listOfParameters><parameter id="p1" value="10" constant="true"/></listOfParameters>
  <listOfReactions>
    <reaction id="J" reversible="false">
      <listOfProducts><speciesReference species="s" stoichiometry="1" constant="true"/></listOfProducts>
      <kineticLaw>
        <math xmlns="http://www.w3.org/1998/Math/MathML"><ci> p1 </ci></math>
        <listOfLocalParameters><localParameter metaid="local_p1" id="p1" value="100"/></listOfLocalParameters>
      </kineticLaw>
    </reaction>
  </listOfReactions>
</comp:modelDefinition>"#,
    );
    let doc = read(&xml).unwrap();
    let law = doc.model.reactions[0].kinetic_law.as_ref().unwrap();
    assert!(law.local_parameters.is_empty());
    assert_eq!(law.math, Some(symbol("A__p1")));
    assert_eq!(doc.model.reactions[0].products[0].species, "A__s");
}

#[test]
fn the_time_of_a_submodel_is_converted() {
    // the submodel runs in minutes, the model in seconds: `time` is `time / 60`, a rate rule
    // is divided by 60
    let xml = composed(
        r#"<model id="main">
  <listOfParameters><parameter id="minute" value="60" constant="true"/></listOfParameters>
  <comp:listOfSubmodels>
    <comp:submodel comp:id="A" comp:modelRef="decay" comp:timeConversionFactor="minute"/>
  </comp:listOfSubmodels>
</model>"#,
        DECAY,
    );
    let doc = read(&xml).unwrap();
    assert_eq!(
        doc.model.rules[0].math(),
        Some(&apply(
            Op::Divide,
            vec![
                apply(Op::Times, vec![symbol("A__k"), symbol("A__x")]),
                symbol("minute")
            ]
        ))
    );
    assert_eq!(
        rule(&doc, "A__y"),
        apply(
            Op::Plus,
            vec![
                symbol("A__x"),
                apply(Op::Divide, vec![Node::CsymbolTime, symbol("minute")])
            ]
        )
    );
}

#[test]
fn an_external_model_definition_is_read_with_its_document() {
    let main = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:comp="http://www.sbml.org/sbml/level3/version1/comp/version1" level="3" version="2" comp:required="true">
  <model id="main">
    <comp:listOfSubmodels><comp:submodel comp:id="A" comp:modelRef="ext"/></comp:listOfSubmodels>
  </model>
  <comp:listOfExternalModelDefinitions>
    <comp:externalModelDefinition comp:id="ext" comp:source="models/decay.xml" comp:modelRef="decay"/>
  </comp:listOfExternalModelDefinitions>
</sbml>"#;
    // the external document is of SBML L3V1 and names a document of its own directory
    let decay = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version1/core" xmlns:comp="http://www.sbml.org/sbml/level3/version1/comp/version1" level="3" version="1" comp:required="true">
  <model id="decay">
    <listOfParameters><parameter id="k" value="2" constant="true"/></listOfParameters>
    <comp:listOfSubmodels><comp:submodel comp:id="B" comp:modelRef="more"/></comp:listOfSubmodels>
  </model>
  <comp:listOfExternalModelDefinitions>
    <comp:externalModelDefinition comp:id="more" comp:source="more.xml"/>
  </comp:listOfExternalModelDefinitions>
</sbml>"#;
    let more = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="more"><listOfParameters><parameter id="q" value="3" constant="true"/></listOfParameters></model>
</sbml>"#;
    let sources = |source: &str| match source {
        "models/decay.xml" => Ok(decay.to_string()),
        "models/more.xml" => Ok(more.to_string()),
        other => Err(format!("no document {other}")),
    };
    let doc = read_with(main, &sources).unwrap();
    assert_eq!(
        parameters(&doc),
        [("A__k", Some(2.0)), ("A__B__q", Some(3.0))]
    );
    // the flattened document is SBML core of the version of the document which is read
    let flat = flatten_comp(main, &sources).unwrap();
    assert!(!flat.contains("comp"), "{flat}");
    assert!(flat.contains("level3/version2/core"), "{flat}");
    assert_eq!(read(&flat).unwrap(), doc);
    // a read of the text alone has no external document
    let ReadError::Xml(message) = read(main).unwrap_err() else {
        panic!()
    };
    assert!(message.contains("read_with"), "{message}");
}

#[test]
fn a_document_without_comp_is_not_flattened() {
    let xml = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m"/></sbml>"#;
    assert_eq!(
        flatten_comp(xml, &|_: &str| Err("none".to_string())).unwrap(),
        xml
    );
}

#[test]
fn a_model_which_contains_itself_is_refused() {
    let xml = composed(
        r#"<model id="main">
  <comp:listOfSubmodels><comp:submodel comp:id="A" comp:modelRef="loop"/></comp:listOfSubmodels>
</model>"#,
        r#"<comp:modelDefinition id="loop">
  <comp:listOfSubmodels><comp:submodel comp:id="again" comp:modelRef="loop"/></comp:listOfSubmodels>
</comp:modelDefinition>"#,
    );
    let ReadError::Xml(message) = read(&xml).unwrap_err() else {
        panic!()
    };
    assert!(message.contains("contains itself"), "{message}");
}
