//! Reading SBML Level 2, versions 1 to 5, which the reader maps onto the model of Level 3.

use sbmlsim_sbml::math::Node;
use sbmlsim_sbml::{Rule, read};

/// The namespaces of the five versions of Level 2.
const NAMESPACES: [(u32, &str); 5] = [
    (1, "http://www.sbml.org/sbml/level2"),
    (2, "http://www.sbml.org/sbml/level2/version2"),
    (3, "http://www.sbml.org/sbml/level2/version3"),
    (4, "http://www.sbml.org/sbml/level2/version4"),
    (5, "http://www.sbml.org/sbml/level2/version5"),
];

/// A Level 2 Version 4 document around the body of its model.
fn level2(body: &str) -> String {
    version_of_level2(4, body)
}

/// A document of the given version of Level 2 around the body of its model.
fn version_of_level2(version: u32, body: &str) -> String {
    let (_, namespace) = NAMESPACES[version as usize - 1];
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="{namespace}" level="2" version="{version}">
  <model id="m">{body}</model>
</sbml>"#
    )
}

#[test]
fn reads_every_version_of_level_2() {
    for (version, _) in NAMESPACES {
        let text = version_of_level2(
            version,
            r#"<listOfParameters><parameter id="k" value="2"/></listOfParameters>"#,
        );
        let doc = read(&text).unwrap_or_else(|error| panic!("L2V{version} is read: {error}"));
        assert_eq!((doc.level, doc.version), (2, version));
        assert_eq!(doc.model.parameters[0].value, Some(2.0));
    }
}

#[test]
fn the_namespace_belongs_to_the_level_and_the_version() {
    let text = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="2" version="4">
      <model id="m"/></sbml>"#;
    let error = read(text).expect_err("the namespace of L3V2 is no document of L2V4");
    assert!(format!("{error}").contains("namespace"), "{error}");
}

#[test]
fn the_built_in_units_of_level_2_are_defined() {
    let doc = read(&level2(
        r#"<listOfCompartments><compartment id="c" size="1"/></listOfCompartments>"#,
    ))
    .unwrap();
    let units = |id: &str| {
        doc.model
            .unit_definitions
            .iter()
            .find(|u| u.id == id)
            .unwrap_or_else(|| panic!("the built in units {id} are defined"))
            .expression
            .clone()
    };
    assert_eq!(units("substance"), "mole");
    // the reader defines them, the model states none of them
    assert!(
        doc.model
            .unit_definitions
            .iter()
            .all(|definition| definition.built_in),
        "the units of the level are the ones of the reader"
    );
    assert_eq!(units("volume"), "litre");
    assert_eq!(units("area"), "metre**2");
    assert_eq!(units("length"), "metre");
    assert_eq!(units("time"), "second");
    // the model takes them as its defaults, as Level 3 states them
    assert_eq!(doc.model.substance_units.as_deref(), Some("substance"));
    assert_eq!(doc.model.volume_units.as_deref(), Some("volume"));
    assert_eq!(doc.model.area_units.as_deref(), Some("area"));
    assert_eq!(doc.model.length_units.as_deref(), Some("length"));
    assert_eq!(doc.model.time_units.as_deref(), Some("time"));
    // the extent of a reaction of Level 2 is a substance
    assert_eq!(doc.model.extent_units.as_deref(), Some("substance"));
}

#[test]
fn a_model_redefines_a_built_in_unit() {
    let doc = read(&level2(
        r#"<listOfUnitDefinitions>
             <unitDefinition id="substance">
               <listOfUnits><unit kind="mole" scale="-3"/></listOfUnits>
             </unitDefinition>
           </listOfUnitDefinitions>"#,
    ))
    .unwrap();
    let substance: Vec<_> = doc
        .model
        .unit_definitions
        .iter()
        .filter(|u| u.id == "substance")
        .collect();
    assert_eq!(substance.len(), 1, "the definition of the model stands");
    assert_eq!(substance[0].expression, "0.001*mole");
    assert!(!substance[0].built_in, "the model states the units itself");
}

#[test]
fn the_defaults_of_a_species_of_level_2() {
    let doc = read(&level2(
        r#"<listOfCompartments><compartment id="c" size="1"/></listOfCompartments>
           <listOfSpecies>
             <species id="s" compartment="c" initialConcentration="2" substanceUnits="substance"/>
           </listOfSpecies>"#,
    ))
    .unwrap();
    let species = &doc.model.species[0];
    assert_eq!(species.initial_concentration, Some(2.0));
    assert_eq!(species.substance_units.as_deref(), Some("substance"));
    assert!(!species.has_only_substance_units);
    assert!(!species.boundary_condition);
    assert!(!species.constant);
    assert_eq!(species.conversion_factor, None, "Level 2 has none");
}

#[test]
fn the_size_and_the_outside_of_a_compartment_of_level_2() {
    let doc = read(&level2(
        r#"<listOfCompartments>
             <compartment id="outer" size="2"/>
             <compartment id="inner" size="1" outside="outer" spatialDimensions="3"/>
           </listOfCompartments>"#,
    ))
    .unwrap();
    assert_eq!(doc.model.compartments[1].size, Some(1.0));
    assert_eq!(doc.model.compartments[1].spatial_dimensions, Some(3.0));
    // and has three dimensions unless it says otherwise, so its unit is the one of a volume
    assert_eq!(doc.model.compartments[0].spatial_dimensions, Some(3.0));
    // a compartment of Level 2 is constant unless it says otherwise
    assert!(doc.model.compartments[1].constant);
    // `outside` is a relation without mathematics, the model is the same without it
    assert_eq!(doc.model.compartments.len(), 2);
}

#[test]
fn a_kinetic_law_of_level_2_holds_its_parameters() {
    let doc = read(&level2(
        r#"<listOfSpecies><species id="s" compartment="c" initialAmount="1"/></listOfSpecies>
           <listOfReactions>
             <reaction id="r">
               <listOfReactants><speciesReference species="s"/></listOfReactants>
               <kineticLaw>
                 <math xmlns="http://www.w3.org/1998/Math/MathML"><ci>k</ci></math>
                 <listOfParameters><parameter id="k" value="3"/></listOfParameters>
               </kineticLaw>
             </reaction>
           </listOfReactions>"#,
    ))
    .unwrap();
    let law = doc.model.reactions[0]
        .kinetic_law
        .as_ref()
        .expect("the reaction has a kinetic law");
    assert_eq!(law.local_parameters.len(), 1);
    assert_eq!(law.local_parameters[0].id, "k");
    assert_eq!(law.local_parameters[0].value, Some(3.0));
}

#[test]
fn a_reaction_of_level_2_is_reversible_and_its_stoichiometry_is_one() {
    let doc = read(&level2(
        r#"<listOfReactions>
             <reaction id="r">
               <listOfReactants><speciesReference species="s"/></listOfReactants>
               <listOfProducts><speciesReference species="p" stoichiometry="2"/></listOfProducts>
             </reaction>
           </listOfReactions>"#,
    ))
    .unwrap();
    let reaction = &doc.model.reactions[0];
    assert!(reaction.reversible, "a reaction is reversible by default");
    assert!(!reaction.fast);
    assert_eq!(reaction.reactants[0].stoichiometry, Some(1.0));
    assert!(reaction.reactants[0].constant);
    assert_eq!(reaction.products[0].stoichiometry, Some(2.0));
}

#[test]
fn the_denominator_of_a_species_reference_divides_its_stoichiometry() {
    let doc = read(&level2(
        r#"<listOfReactions>
             <reaction id="r">
               <listOfReactants>
                 <speciesReference species="s" stoichiometry="3" denominator="2"/>
               </listOfReactants>
             </reaction>
           </listOfReactions>"#,
    ))
    .unwrap();
    assert_eq!(doc.model.reactions[0].reactants[0].stoichiometry, Some(1.5));
}

#[test]
fn a_constant_stoichiometry_math_is_the_stoichiometry() {
    let doc = read(&level2(
        r#"<listOfReactions>
             <reaction id="r">
               <listOfProducts>
                 <speciesReference species="p">
                   <stoichiometryMath>
                     <math xmlns="http://www.w3.org/1998/Math/MathML"><cn>2</cn></math>
                   </stoichiometryMath>
                 </speciesReference>
               </listOfProducts>
             </reaction>
           </listOfReactions>"#,
    ))
    .unwrap();
    let reference = &doc.model.reactions[0].products[0];
    assert_eq!(reference.stoichiometry, Some(2.0));
    assert!(reference.constant);
    assert!(doc.model.rules.is_empty(), "a number needs no rule");
}

#[test]
fn a_stoichiometry_math_of_a_formula_becomes_an_assignment_rule() {
    let doc = read(&level2(
        r#"<listOfParameters><parameter id="k" value="2"/></listOfParameters>
           <listOfReactions>
             <reaction id="r">
               <listOfProducts>
                 <speciesReference species="p">
                   <stoichiometryMath>
                     <math xmlns="http://www.w3.org/1998/Math/MathML">
                       <apply><times/><ci>k</ci><cn>2</cn></apply>
                     </math>
                   </stoichiometryMath>
                 </speciesReference>
               </listOfProducts>
             </reaction>
           </listOfReactions>"#,
    ))
    .unwrap();
    let reference = &doc.model.reactions[0].products[0];
    assert!(!reference.constant, "the stoichiometry changes in time");
    assert_eq!(reference.stoichiometry, None, "the rule gives it");
    let id = reference
        .id
        .as_deref()
        .expect("the reference the rule assigns has an id");
    match &doc.model.rules[..] {
        [Rule::Assignment { variable, math, .. }] => {
            assert_eq!(variable, id);
            assert!(matches!(math, Some(Node::Apply { .. })), "{math:?}");
        }
        rules => panic!("one assignment rule of the stoichiometry, not {rules:?}"),
    }
}

#[test]
fn the_id_of_a_species_reference_carries_its_stoichiometry_math() {
    let doc = read(&level2(
        r#"<listOfParameters><parameter id="k" value="2"/></listOfParameters>
           <listOfReactions>
             <reaction id="r">
               <listOfProducts>
                 <speciesReference id="given" species="p">
                   <stoichiometryMath>
                     <math xmlns="http://www.w3.org/1998/Math/MathML"><ci>k</ci></math>
                   </stoichiometryMath>
                 </speciesReference>
               </listOfProducts>
             </reaction>
           </listOfReactions>"#,
    ))
    .unwrap();
    assert_eq!(
        doc.model.reactions[0].products[0].id.as_deref(),
        Some("given")
    );
    match &doc.model.rules[..] {
        [Rule::Assignment { variable, .. }] => assert_eq!(variable, "given"),
        rules => panic!("the rule assigns the id of the reference, not {rules:?}"),
    }
}

#[test]
fn an_event_of_level_2_uses_the_values_from_the_trigger_time() {
    let event = |version, attribute: &str| {
        let body = format!(
            r#"<listOfEvents>
                 <event id="e"{attribute}>
                   <trigger>
                     <math xmlns="http://www.w3.org/1998/Math/MathML"><true/></math>
                   </trigger>
                   <listOfEventAssignments>
                     <eventAssignment variable="s">
                       <math xmlns="http://www.w3.org/1998/Math/MathML"><cn>1</cn></math>
                     </eventAssignment>
                   </listOfEventAssignments>
                 </event>
               </listOfEvents>"#
        );
        read(&version_of_level2(version, &body))
            .unwrap()
            .model
            .events[0]
            .clone()
    };
    // the attribute arrived in Version 4; before it, the values are the ones of the trigger time
    assert!(event(1, "").use_values_from_trigger_time);
    assert!(event(4, "").use_values_from_trigger_time);
    assert!(!event(4, r#" useValuesFromTriggerTime="false""#).use_values_from_trigger_time);
}

#[test]
fn the_trigger_of_an_event_of_level_2_is_persistent_with_the_initial_value_true() {
    let doc = read(&level2(
        r#"<listOfEvents>
             <event id="e">
               <trigger><math xmlns="http://www.w3.org/1998/Math/MathML"><true/></math></trigger>
             </event>
           </listOfEvents>"#,
    ))
    .unwrap();
    let trigger = doc.model.events[0]
        .trigger
        .as_ref()
        .expect("the event has a trigger");
    assert!(trigger.initial_value, "Level 2 has no other reading");
    assert!(trigger.persistent, "Level 2 knows no persistence");
}

#[test]
fn a_parameter_a_rate_rule_determines_is_no_constant_without_the_attribute() {
    let doc = read(&level2(
        r#"<listOfParameters><parameter id="p" value="1"/></listOfParameters>
           <listOfRules>
             <rateRule variable="p">
               <math xmlns="http://www.w3.org/1998/Math/MathML"><cn>1</cn></math>
             </rateRule>
           </listOfRules>"#,
    ))
    .unwrap();
    // the default of Level 2 is a constant, which what a rule determines cannot be
    assert!(!doc.model.parameters[0].constant);
}

#[test]
fn a_parameter_an_event_assigns_is_no_constant_without_the_attribute() {
    let doc = read(&level2(
        r#"<listOfParameters><parameter id="p" value="1"/></listOfParameters>
           <listOfEvents>
             <event id="e">
               <trigger><math xmlns="http://www.w3.org/1998/Math/MathML"><true/></math></trigger>
               <listOfEventAssignments>
                 <eventAssignment variable="p">
                   <math xmlns="http://www.w3.org/1998/Math/MathML"><cn>2</cn></math>
                 </eventAssignment>
               </listOfEventAssignments>
             </event>
           </listOfEvents>"#,
    ))
    .unwrap();
    assert!(!doc.model.parameters[0].constant);
}

#[test]
fn a_parameter_an_algebraic_rule_reads_keeps_the_default_of_level_2() {
    let doc = read(&level2(
        r#"<listOfParameters><parameter id="k" value="2"/></listOfParameters>
           <listOfRules>
             <algebraicRule>
               <math xmlns="http://www.w3.org/1998/Math/MathML">
                 <apply><minus/><ci>x</ci><ci>k</ci></apply>
               </math>
             </algebraicRule>
           </listOfRules>"#,
    ))
    .unwrap();
    // Level 2 states the attribute, so the reader takes it as it stands, unlike Level 1
    assert!(doc.model.parameters[0].constant);
}
