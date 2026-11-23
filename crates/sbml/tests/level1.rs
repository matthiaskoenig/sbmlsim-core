//! Reading SBML Level 1, which names its elements with `name` and holds its mathematics as
//! infix formulas; the reader gives the model of Level 3 for it as well.

use sbmlsim_sbml::math::{Node, Op};
use sbmlsim_sbml::{Rule, read};

/// A Level 1 document of the given version around the body of its model.
fn level1(version: u32, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level1" level="1" version="{version}">
  <model name="m">{body}</model>
</sbml>"#
    )
}

#[test]
fn reads_both_versions_of_level_1() {
    for version in [1, 2] {
        let doc = read(&level1(
            version,
            r#"<listOfParameters><parameter name="k" value="2"/></listOfParameters>"#,
        ))
        .unwrap_or_else(|error| panic!("L1V{version} is read: {error}"));
        assert_eq!((doc.level, doc.version), (1, version));
        // the name of an element of Level 1 is its identifier
        assert_eq!(doc.model.id.as_deref(), Some("m"));
        assert_eq!(doc.model.parameters[0].id, "k");
        assert_eq!(doc.model.parameters[0].value, Some(2.0));
        assert!(doc.model.parameters[0].constant);
    }
}

#[test]
fn the_volume_of_a_compartment_is_its_size() {
    let doc = read(&level1(
        2,
        r#"<listOfCompartments><compartment name="c" volume="2.5"/></listOfCompartments>"#,
    ))
    .unwrap();
    assert_eq!(doc.model.compartments[0].id, "c");
    assert_eq!(doc.model.compartments[0].size, Some(2.5));
    assert!(doc.model.compartments[0].constant);
}

#[test]
fn a_species_of_version_1_is_a_specie() {
    let doc = read(&level1(
        1,
        r#"<listOfSpecies>
             <specie name="s" compartment="c" initialAmount="3" units="substance"/>
           </listOfSpecies>"#,
    ))
    .unwrap();
    let species = &doc.model.species[0];
    assert_eq!(species.id, "s");
    assert_eq!(species.compartment, "c");
    assert_eq!(species.initial_amount, Some(3.0));
    // the `units` of a species of Level 1 are its substance units
    assert_eq!(species.substance_units.as_deref(), Some("substance"));
    assert!(!species.has_only_substance_units);
}

#[test]
fn a_species_of_version_2_is_a_species() {
    let doc = read(&level1(
        2,
        r#"<listOfSpecies>
             <species name="s" compartment="c" initialAmount="3" boundaryCondition="true"/>
           </listOfSpecies>"#,
    ))
    .unwrap();
    assert_eq!(doc.model.species[0].id, "s");
    assert!(doc.model.species[0].boundary_condition);
}

#[test]
fn the_built_in_units_of_level_1_are_defined() {
    let doc = read(&level1(2, "")).unwrap();
    let ids: Vec<&str> = doc
        .model
        .unit_definitions
        .iter()
        .map(|u| u.id.as_str())
        .collect();
    assert_eq!(ids, ["substance", "volume", "area", "length", "time"]);
    assert_eq!(doc.model.substance_units.as_deref(), Some("substance"));
}

#[test]
fn a_unit_definition_of_level_1_is_named() {
    let doc = read(&level1(
        2,
        r#"<listOfUnitDefinitions>
             <unitDefinition name="mmol">
               <listOfUnits><unit kind="mole" scale="-3"/></listOfUnits>
             </unitDefinition>
           </listOfUnitDefinitions>"#,
    ))
    .unwrap();
    let mmol = doc
        .model
        .unit_definitions
        .iter()
        .find(|u| u.id == "mmol")
        .expect("the definition is named by its `name`");
    assert_eq!(mmol.expression, "0.001*mole");
}

#[test]
fn the_formula_of_a_kinetic_law_is_its_math() {
    let doc = read(&level1(
        2,
        r#"<listOfReactions>
             <reaction name="r">
               <listOfReactants><speciesReference species="s" stoichiometry="2"/></listOfReactants>
               <kineticLaw formula="k * s">
                 <listOfParameters><parameter name="k" value="1"/></listOfParameters>
               </kineticLaw>
             </reaction>
           </listOfReactions>"#,
    ))
    .unwrap();
    let reaction = &doc.model.reactions[0];
    assert_eq!(reaction.id, "r");
    assert!(reaction.reversible, "a reaction of Level 1 is reversible");
    assert_eq!(reaction.reactants[0].species, "s");
    assert_eq!(reaction.reactants[0].stoichiometry, Some(2.0));
    let law = reaction.kinetic_law.as_ref().expect("the law is read");
    assert_eq!(law.local_parameters[0].id, "k");
    assert_eq!(
        law.math,
        Some(Node::Apply {
            op: Op::Times,
            args: vec![Node::Symbol("k".into()), Node::Symbol("s".into())],
        })
    );
}

#[test]
fn a_species_reference_of_version_1_names_a_specie() {
    let doc = read(&level1(
        1,
        r#"<listOfReactions>
             <reaction name="r">
               <listOfProducts><specieReference specie="s" stoichiometry="3" denominator="2"/></listOfProducts>
             </reaction>
           </listOfReactions>"#,
    ))
    .unwrap();
    assert_eq!(doc.model.reactions[0].products[0].species, "s");
    assert_eq!(doc.model.reactions[0].products[0].stoichiometry, Some(1.5));
}

#[test]
fn the_rules_of_level_1_are_named_after_what_they_determine() {
    let doc = read(&level1(
        2,
        r#"<listOfRules>
             <parameterRule name="p" formula="2 * k"/>
             <speciesConcentrationRule species="s" formula="k" type="rate"/>
             <compartmentVolumeRule compartment="c" formula="1"/>
             <algebraicRule formula="p - 1"/>
           </listOfRules>"#,
    ))
    .unwrap();
    match &doc.model.rules[..] {
        [
            Rule::Assignment { variable: p, .. },
            Rule::Rate { variable: s, .. },
            Rule::Assignment { variable: c, .. },
            Rule::Algebraic { math, .. },
        ] => {
            assert_eq!(p, "p");
            // `type="rate"` makes a rule the rate of its variable
            assert_eq!(s, "s");
            assert_eq!(c, "c");
            assert!(matches!(math, Some(Node::Apply { .. })), "{math:?}");
        }
        rules => panic!("four rules of Level 1, not {rules:?}"),
    }
}

#[test]
fn a_concentration_rule_of_version_1_names_a_specie() {
    let doc = read(&level1(
        1,
        r#"<listOfRules>
             <specieConcentrationRule specie="s" formula="2"/>
           </listOfRules>"#,
    ))
    .unwrap();
    match &doc.model.rules[..] {
        [Rule::Assignment { variable, math, .. }] => {
            assert_eq!(variable, "s");
            assert_eq!(math, &Some(Node::number(2.0)));
        }
        rules => panic!("the rule of the specie, not {rules:?}"),
    }
}

#[test]
fn a_formula_which_is_no_formula_names_the_element_it_is_on() {
    let error = read(&level1(
        2,
        r#"<listOfRules><parameterRule name="p" formula="2 +"/></listOfRules>"#,
    ))
    .expect_err("the formula is no formula");
    let message = format!("{error}");
    assert!(message.contains("parameterRule"), "{message}");
    assert!(message.contains("formula"), "{message}");
}

#[test]
fn a_quantity_a_rule_determines_is_no_constant() {
    let doc = read(&level1(
        2,
        r#"<listOfCompartments><compartment name="c" volume="1"/></listOfCompartments>
           <listOfParameters>
             <parameter name="p" value="1"/>
             <parameter name="k" value="2"/>
           </listOfParameters>
           <listOfRules>
             <parameterRule name="p" formula="k" type="rate"/>
             <compartmentVolumeRule compartment="c" formula="p"/>
           </listOfRules>"#,
    ))
    .unwrap();
    let parameter = |id: &str| {
        doc.model
            .parameters
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("the parameter {id}"))
    };
    // Level 1 has no `constant`, so what a rule determines is no constant
    assert!(!parameter("p").constant);
    assert!(!doc.model.compartments[0].constant);
    assert!(parameter("k").constant, "nothing determines it");
}

#[test]
fn a_quantity_an_algebraic_rule_reads_is_no_constant() {
    let doc = read(&level1(
        2,
        r#"<listOfParameters>
             <parameter name="kf"/>
             <parameter name="kr" value="0.25"/>
           </listOfParameters>
           <listOfRules><algebraicRule formula="kf + -0.75"/></listOfRules>"#,
    ))
    .unwrap();
    // the rule determines `kf`, which has no value of its own
    assert!(!doc.model.parameters[0].constant);
    assert!(
        doc.model.parameters[1].constant,
        "the rule does not read it"
    );
}
