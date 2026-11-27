//! Core SBML elements with authored attributes and level-specific defaults.

use crate::{MATHML, ReadError, elements, math, metadata, packages, xml};
use roxmltree::Node;
use sbmlsim_document::*;
use sha1::{Digest, Sha1};

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    elements(node).find(|n| {
        n.tag_name().namespace() == node.document().root_element().tag_name().namespace()
            && n.tag_name().name() == name
    })
}
fn list<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Vec<Node<'a, 'input>> {
    child(node, name)
        .map(|n| {
            elements(n)
                .filter(|x| {
                    x.tag_name().namespace()
                        == node.document().root_element().tag_name().namespace()
                        && !matches!(x.tag_name().name(), "notes" | "annotation")
                })
                .collect()
        })
        .unwrap_or_default()
}
fn string(node: Node, key: &str) -> Option<String> {
    node.attribute(key).map(str::to_owned)
}
fn number(node: Node, key: &str) -> Option<f64> {
    node.attribute(key).and_then(|v| match v {
        "INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        _ => v.parse().ok(),
    })
}
fn flag(node: Node, key: &str) -> Option<bool> {
    node.attribute(key).and_then(|v| match v {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    })
}
fn legacy_flag(node: Node, key: &str, level: u32, default: bool) -> Option<bool> {
    flag(node, key).or_else(|| (level == 2).then_some(default))
}
fn model_math(node: Node) -> Result<Option<Ast>, ReadError> {
    if node.document().root_element().attribute("level") == Some("1")
        && let Some(formula) = node.attribute("formula")
    {
        return Ok(crate::formula::read_formula(formula).unwrap_or(None));
    }
    elements(node)
        .find(|n| n.tag_name().namespace() == Some(MATHML) && n.tag_name().name() == "math")
        .map(math::from_node)
        .transpose()
        .map(Option::flatten)
}

struct Reader<'a> {
    level: u32,
    positions: &'a metadata::Positions<'a>,
}
impl Reader<'_> {
    fn base(&self, node: Node) -> Element {
        metadata::element(node, self.level, true, self.positions)
    }
    fn function(&self, node: Node) -> Result<FunctionDefinition, ReadError> {
        Ok(FunctionDefinition {
            element: self.base(node),
            math: model_math(node)?,
        })
    }
    fn unit_definition(&self, node: Node) -> UnitDefinition {
        UnitDefinition {
            element: self.base(node),
            units: list(node, "listOfUnits")
                .into_iter()
                .map(|n| Unit {
                    kind: string(n, "kind"),
                    exponent: number(n, "exponent").or_else(|| (self.level < 3).then_some(1.0)),
                    scale: n
                        .attribute("scale")
                        .and_then(|s| s.parse().ok())
                        .or_else(|| (self.level < 3).then_some(0)),
                    multiplier: number(n, "multiplier").or_else(|| (self.level < 3).then_some(1.0)),
                })
                .collect(),
        }
    }
    fn compartment(&self, node: Node) -> Compartment {
        Compartment {
            element: self.base(node),
            spatial_dimensions: number(node, "spatialDimensions")
                .or_else(|| (self.level < 3).then_some(3.0)),
            size: number(node, "size").or_else(|| number(node, "volume")),
            constant: legacy_flag(node, "constant", self.level, true),
            units: string(node, "units"),
        }
    }
    fn species(&self, node: Node) -> Species {
        Species {
            element: self.base(node),
            compartment: node.attribute("compartment").unwrap_or("").to_owned(),
            initial_amount: number(node, "initialAmount"),
            initial_concentration: number(node, "initialConcentration"),
            substance_units: string(node, "substanceUnits").or_else(|| string(node, "units")),
            spatial_size_units: (self.level == 2)
                .then(|| string(node, "spatialSizeUnits"))
                .flatten(),
            has_only_substance_units: legacy_flag(node, "hasOnlySubstanceUnits", self.level, false),
            boundary_condition: flag(node, "boundaryCondition")
                .or_else(|| (self.level < 3).then_some(false)),
            constant: legacy_flag(node, "constant", self.level, false),
            conversion_factor: string(node, "conversionFactor"),
        }
    }
    fn parameter(&self, node: Node) -> Parameter {
        Parameter {
            element: self.base(node),
            value: number(node, "value"),
            units: string(node, "units"),
            constant: legacy_flag(node, "constant", self.level, true),
        }
    }
    fn initial(&self, node: Node) -> Result<InitialAssignment, ReadError> {
        Ok(InitialAssignment {
            element: self.base(node),
            symbol: node.attribute("symbol").unwrap_or("").to_owned(),
            math: model_math(node)?,
        })
    }
    fn rule(&self, node: Node) -> Result<Rule, ReadError> {
        Ok(Rule {
            element: self.base(node),
            kind: match node.tag_name().name() {
                "assignmentRule" => RuleKind::Assignment,
                "speciesConcentrationRule" | "compartmentVolumeRule" | "parameterRule" => {
                    if node.attribute("type") == Some("rate") {
                        RuleKind::Rate
                    } else {
                        RuleKind::Assignment
                    }
                }
                "rateRule" => RuleKind::Rate,
                _ => RuleKind::Algebraic,
            },
            variable: string(node, "variable")
                .or_else(|| string(node, "species"))
                .or_else(|| string(node, "specie"))
                .or_else(|| string(node, "compartment"))
                .or_else(|| (self.level == 1).then(|| string(node, "name")).flatten()),
            math: model_math(node)?,
        })
    }
    fn constraint(&self, node: Node) -> Result<Constraint, ReadError> {
        Ok(Constraint {
            element: self.base(node),
            math: model_math(node)?,
            message: child(node, "message").map(|n| xml::fragment(n, self.level)),
        })
    }
    fn reference(&self, node: Node) -> Result<SpeciesReference, ReadError> {
        Ok(SpeciesReference {
            element: self.base(node),
            species: node
                .attribute("species")
                .or_else(|| node.attribute("specie"))
                .unwrap_or("")
                .to_owned(),
            stoichiometry: number(node, "stoichiometry").unwrap_or(if self.level < 3 {
                1.0
            } else {
                f64::NAN
            }),
            stoichiometry_set: (node.attribute("stoichiometry").is_some() || self.level < 3)
                && child(node, "stoichiometryMath").is_none(),
            stoichiometry_math: child(node, "stoichiometryMath").is_some(),
            stoichiometry_math_ast: child(node, "stoichiometryMath")
                .map(model_math)
                .transpose()?
                .flatten(),
            constant: flag(node, "constant"),
        })
    }
    fn local(&self, node: Node) -> LocalParameter {
        LocalParameter {
            element: self.base(node),
            value: number(node, "value"),
            units: string(node, "units"),
        }
    }
    fn law(&self, node: Node) -> Result<KineticLaw, ReadError> {
        Ok(KineticLaw {
            element: self.base(node),
            math: model_math(node)?,
            local_parameters: list(
                node,
                if self.level < 3 {
                    "listOfParameters"
                } else {
                    "listOfLocalParameters"
                },
            )
            .into_iter()
            .map(|n| self.local(n))
            .collect(),
        })
    }
    fn reaction(&self, node: Node) -> Result<Reaction, ReadError> {
        Ok(Reaction {
            element: self.base(node),
            reversible: flag(node, "reversible").or_else(|| (self.level < 3).then_some(true)),
            fast: flag(node, "fast"),
            compartment: string(node, "compartment"),
            reactants: list(node, "listOfReactants")
                .into_iter()
                .map(|n| self.reference(n))
                .collect::<Result<_, _>>()?,
            products: list(node, "listOfProducts")
                .into_iter()
                .map(|n| self.reference(n))
                .collect::<Result<_, _>>()?,
            modifiers: list(node, "listOfModifiers")
                .into_iter()
                .map(|n| ModifierSpeciesReference {
                    element: self.base(n),
                    species: n.attribute("species").unwrap_or("").to_owned(),
                })
                .collect(),
            kinetic_law: child(node, "kineticLaw").map(|n| self.law(n)).transpose()?,
        })
    }
    fn trigger(&self, node: Node) -> Result<Trigger, ReadError> {
        Ok(Trigger {
            element: self.base(node),
            math: model_math(node)?,
            initial_value: flag(node, "initialValue"),
            persistent: flag(node, "persistent"),
        })
    }
    fn math_element(&self, node: Node) -> Result<MathElement, ReadError> {
        Ok(MathElement {
            element: self.base(node),
            math: model_math(node)?,
        })
    }
    fn assignment(&self, node: Node) -> Result<EventAssignment, ReadError> {
        Ok(EventAssignment {
            element: self.base(node),
            variable: node.attribute("variable").unwrap_or("").to_owned(),
            math: model_math(node)?,
        })
    }
    fn event(&self, node: Node) -> Result<Event, ReadError> {
        Ok(Event {
            element: self.base(node),
            use_values_from_trigger_time: flag(node, "useValuesFromTriggerTime")
                .or_else(|| (self.level < 3).then_some(true)),
            trigger: child(node, "trigger")
                .map(|n| self.trigger(n))
                .transpose()?,
            priority: child(node, "priority")
                .map(|n| self.math_element(n))
                .transpose()?,
            delay: child(node, "delay")
                .map(|n| self.math_element(n))
                .transpose()?,
            event_assignments: list(node, "listOfEventAssignments")
                .into_iter()
                .map(|n| self.assignment(n))
                .collect::<Result<_, _>>()?,
        })
    }
    fn model(&self, node: Node) -> Result<Model, ReadError> {
        Ok(Model {
            element: metadata::element(node, self.level, false, self.positions),
            units: ModelUnits {
                substance: string(node, "substanceUnits"),
                time: string(node, "timeUnits"),
                volume: string(node, "volumeUnits"),
                area: string(node, "areaUnits"),
                length: string(node, "lengthUnits"),
                extent: string(node, "extentUnits"),
            },
            conversion_factor: string(node, "conversionFactor"),
            function_definitions: list(node, "listOfFunctionDefinitions")
                .into_iter()
                .map(|n| self.function(n))
                .collect::<Result<_, _>>()?,
            unit_definitions: list(node, "listOfUnitDefinitions")
                .into_iter()
                .map(|n| self.unit_definition(n))
                .collect(),
            compartments: list(node, "listOfCompartments")
                .into_iter()
                .map(|n| self.compartment(n))
                .collect(),
            species: list(node, "listOfSpecies")
                .into_iter()
                .map(|n| self.species(n))
                .collect(),
            parameters: list(node, "listOfParameters")
                .into_iter()
                .map(|n| self.parameter(n))
                .collect(),
            initial_assignments: list(node, "listOfInitialAssignments")
                .into_iter()
                .map(|n| self.initial(n))
                .collect::<Result<_, _>>()?,
            rules: list(node, "listOfRules")
                .into_iter()
                .map(|n| self.rule(n))
                .collect::<Result<_, _>>()?,
            constraints: list(node, "listOfConstraints")
                .into_iter()
                .map(|n| self.constraint(n))
                .collect::<Result<_, _>>()?,
            reactions: list(node, "listOfReactions")
                .into_iter()
                .map(|n| self.reaction(n))
                .collect::<Result<_, _>>()?,
            events: list(node, "listOfEvents")
                .into_iter()
                .map(|n| self.event(n))
                .collect::<Result<_, _>>()?,
        })
    }
}

/// Read an SBML document without accessing native libraries, files or external resources.
pub fn read(source: &[u8]) -> Result<Document, ReadError> {
    let decoded = crate::encoding::decode(source)?;
    let source = decoded.as_ref();
    let xml = roxmltree::Document::parse(source).map_err(|e| ReadError(e.to_string()))?;
    let root = xml.root_element();
    for node in xml.descendants().filter(|n| {
        n.is_element()
            && matches!(
                n.tag_name().namespace(),
                Some("http://biomodels.net/biology-qualifiers/")
                    | Some("http://biomodels.net/model-qualifiers/")
            )
    }) {
        if node
            .ancestors()
            .filter(|n| {
                n.is_element()
                    && matches!(
                        n.tag_name().namespace(),
                        Some("http://biomodels.net/biology-qualifiers/")
                            | Some("http://biomodels.net/model-qualifiers/")
                    )
            })
            .take(129)
            .count()
            > 128
        {
            return Err(ReadError(
                "Controlled annotation nesting exceeds 128 qualifiers.".into(),
            ));
        }
    }
    if root.tag_name().name() != "sbml" {
        return Err(ReadError("The source is not an SBML document.".into()));
    }
    let level: u32 = root
        .attribute("level")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| ReadError("The SBML level is missing.".into()))?;
    let version: u32 = root
        .attribute("version")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| ReadError("The SBML version is missing.".into()))?;
    let valid = match level {
        1 => (1..=2).contains(&version),
        2 => (1..=5).contains(&version),
        3 => (1..=2).contains(&version),
        _ => false,
    };
    let namespace = root.tag_name().namespace().unwrap_or("");
    if !valid || !namespace.starts_with("http://www.sbml.org/sbml/level") {
        return Err(ReadError(format!(
            "Unsupported SBML level {level} version {version} namespace {namespace:?}."
        )));
    }
    let positions = metadata::Positions::new(source);
    let reader = Reader {
        level,
        positions: &positions,
    };
    let mut packages: Vec<_> = root
        .namespaces()
        .filter_map(|ns| {
            if ns.uri() == namespace {
                return None;
            }
            let parts: Vec<_> = ns.uri().split('/').collect();
            let index = parts
                .iter()
                .position(|s| *s == "core")
                .map(|i| i + 1)
                .unwrap_or(6);
            let name = *parts.get(index)?;
            let version = parts
                .get(index + 1)?
                .strip_prefix("version")?
                .parse()
                .ok()?;
            let prefix = ns.name().unwrap_or(name);
            Some(Package {
                name: name.to_owned(),
                prefix: prefix.to_owned(),
                version,
                required: root
                    .attribute((ns.uri(), "required"))
                    .is_some_and(|v| matches!(v, "true" | "1")),
            })
        })
        .collect();
    if level == 2 {
        for name in ["layout", "render"] {
            if !packages.iter().any(|p| p.prefix == name) {
                packages.push(Package {
                    name: name.into(),
                    prefix: name.into(),
                    version: 1,
                    required: false,
                });
            }
        }
    }
    let mut document = Document {
        level,
        version,
        element: metadata::element(root, level, false, &positions),
        packages,
        model: child(root, "model").map(|n| reader.model(n)).transpose()?,
        main_model_fallback: None,
        model_definitions: elements(root)
            .filter(|n| {
                packages::name(n.tag_name().namespace()) == Some("comp")
                    && n.tag_name().name() == "listOfModelDefinitions"
            })
            .flat_map(elements)
            .filter(|n| n.tag_name().name() == "modelDefinition")
            .map(|n| reader.model(n))
            .collect::<Result<_, _>>()?,
    };
    if document
        .model
        .as_ref()
        .is_some_and(|m| m.element.id.is_none() && m.element.meta_id.is_none())
        && document
            .model_definitions
            .iter()
            .any(|m| m.element.id.as_deref().or(m.element.meta_id.as_deref()) == Some("model"))
        && let Some(node) = child(root, "model")
    {
        document.main_model_fallback = Some(
            Sha1::digest(xml::fragment(node, level).as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        );
    }
    packages::apply(&xml, level, &mut document, &positions)?;
    Ok(document)
}
