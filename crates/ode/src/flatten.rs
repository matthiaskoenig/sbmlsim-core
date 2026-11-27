//! Confined comp expansion. Only owned documents supplied by the caller resolve.
use super::ModelResolver;
use super::ast::{name, node, number, product};
use sbmlsim_document::packages::{attr, children};
use sbmlsim_document::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
struct Target<'a> {
    instance: usize,
    element: &'a Element,
    child: Option<usize>,
}
impl Target<'_> {
    fn key(self) -> (usize, usize) {
        (self.instance, std::ptr::from_ref(self.element) as usize)
    }
}
struct Instance<'a> {
    model: &'a Model,
    prefix: String,
    children: HashMap<&'a str, usize>,
    sids: HashMap<&'a str, Target<'a>>,
    units: HashMap<&'a str, Target<'a>>,
    meta: HashMap<&'a str, Target<'a>>,
    ports: HashMap<&'a str, &'a Extension>,
    parent: Option<(usize, &'a Extension)>,
}
fn core(model: &Model) -> Vec<&Element> {
    let mut out = vec![&model.element];
    macro_rules! add {
        ($list:expr) => {
            out.extend(($list).iter().map(|e| &e.element));
        };
    }
    add!(&model.function_definitions);
    add!(&model.unit_definitions);
    add!(&model.compartments);
    add!(&model.species);
    add!(&model.parameters);
    add!(&model.initial_assignments);
    add!(&model.rules);
    add!(&model.constraints);
    for r in &model.reactions {
        out.push(&r.element);
        add!(&r.reactants);
        add!(&r.products);
        add!(&r.modifiers);
        if let Some(law) = &r.kinetic_law {
            out.push(&law.element);
            add!(&law.local_parameters);
        }
    }
    for e in &model.events {
        out.push(&e.element);
        add!(&e.trigger);
        add!(&e.priority);
        add!(&e.delay);
        add!(&e.event_assignments);
    }
    out
}
fn ext<'a>(element: &'a Element, name: &str) -> Option<&'a Extension> {
    element
        .extensions
        .iter()
        .find(|e| e.package == "comp" && e.name == name)
}
fn id(element: &Element) -> &str {
    element.id.as_deref().unwrap_or("")
}
fn external_error(
    document: &Document,
    context: Option<(&dyn ModelResolver<'_>, usize)>,
) -> Option<String> {
    let external = children(&document.element, "comp", "listOfExternalModelDefinitions")
        .into_iter()
        .filter(|e| context.is_none_or(|(c, entry)| c.model(entry, id(&e.element)).is_none()))
        .collect::<Vec<_>>();
    if external.is_empty() {
        return None;
    }
    let general = "The external model referenced in this model could not be resolved. \n";
    let mut errors = Vec::new();
    let mut uses = Vec::new();
    if let Some(model) = &document.model {
        let submodels = children(&model.element, "comp", "listOfSubmodels");
        let mut references = Vec::new();
        for element in core(model) {
            let mut stack: Vec<_> = element.extensions.iter().collect();
            while let Some(extension) = stack.pop() {
                if extension.package == "comp"
                    && matches!(extension.name.as_str(), "replacedElement" | "replacedBy")
                {
                    references.push(extension);
                }
                stack.extend(&extension.children);
            }
        }
        references.sort_by_key(|r| (r.element.line, r.element.column));
        for reference in references {
            if let Some(submodel) = submodels
                .iter()
                .find(|s| s.element.id.as_deref() == attr(reference, "submodelRef"))
                && let Some(emd) = external
                    .iter()
                    .find(|e| e.element.id.as_deref() == attr(submodel, "modelRef"))
            {
                uses.push((*submodel, *emd));
            }
        }
    }
    // Replacement resolution visits the authored references in each of the two flattening passes.
    for _ in 0..2 {
        for (submodel, emd) in &uses {
            let eid = id(&emd.element);
            let source = attr(emd, "source").unwrap_or("");
            errors.push(format!("  E1090101: {general} In ExternalModelDefinition::getReferencedModel, unable to resolve the external model definition '{eid}': could not the resolve 'source' attribute '{source}' as a valid SBML Document."));
            errors.push(format!("  E1020615: The value of a 'comp:modelRef' attribute on a <submodel> must be the identifier of a <model>, <modelDefinition>, or <externalModelDefinition> object in the same SBML object as the <submodel>.\nReference: L3V1 Comp V1 Section 3.5.1\n In Submodel::instantiate, unable to instantiate submodel '{}' because the external model definition it referenced (model '{eid}') could not be resolved.",id(&submodel.element)));
        }
    }
    for emd in external {
        errors.push(format!("  E1090101: {general} The <externalModelDefinition> with the id '{}' refers to a source '{}' that cannot be accessed from here. Further checks relating to this document cannot be performed.",id(&emd.element),attr(emd,"source").unwrap_or("")));
    }
    Some(format!(
        "SBML could not be flattened due to errors:\n{}",
        errors.join("\n")
    ))
}
type PendingInstance<'a> = (
    &'a Model,
    usize,
    String,
    Option<(usize, &'a Extension)>,
    Vec<usize>,
);

struct Flatten<'a> {
    instances: Vec<Instance<'a>>,
    aliases: HashMap<(usize, usize), Target<'a>>,
    removed: HashSet<(usize, usize)>,
    names: HashMap<(usize, usize), String>,
    conversion: HashMap<(usize, usize), (usize, String)>,
}
impl<'a> Flatten<'a> {
    fn new(
        document: &'a Document,
        context: Option<(&dyn ModelResolver<'a>, usize)>,
    ) -> Result<Self, String> {
        let model = document
            .model
            .as_ref()
            .ok_or("The SBML document has no model.")?;
        let definitions: HashMap<_, _> = document
            .models()
            .filter_map(|m| m.element.id.as_deref().map(|id| (id, m)))
            .collect();
        let mut this = Self {
            instances: vec![],
            aliases: HashMap::new(),
            removed: HashSet::new(),
            names: HashMap::new(),
            conversion: HashMap::new(),
        };
        let mut pending: Vec<PendingInstance> = vec![(
            model,
            context.map_or(0, |(_, entry)| entry),
            String::new(),
            None,
            Vec::<usize>::new(),
        )];
        let mut count = 0;
        while let Some((model, entry, prefix, parent, mut ancestry)) = pending.pop() {
            let identity = std::ptr::from_ref(model) as usize;
            if ancestry.contains(&identity) {
                return Err("The comp submodels instantiate their model in a cycle.".into());
            }
            ancestry.push(identity);
            count += core(model).len();
            if count > context.map_or(10_000, |(c, _)| c.budget()) {
                return Err("The expanded comp model exceeds 10000 SBML elements.".into());
            }
            let i = this.instances.len();
            if let Some((p, submodel)) = parent {
                this.instances[p].children.insert(id(&submodel.element), i);
            }
            let ports = children(&model.element, "comp", "listOfPorts")
                .into_iter()
                .filter_map(|p| p.element.id.as_deref().map(|id| (id, p)))
                .collect();
            this.instances.push(Instance {
                model,
                prefix: prefix.clone(),
                children: HashMap::new(),
                sids: HashMap::new(),
                units: HashMap::new(),
                meta: HashMap::new(),
                ports,
                parent,
            });
            for submodel in children(&model.element, "comp", "listOfSubmodels")
                .into_iter()
                .rev()
            {
                let sid = id(&submodel.element);
                let model_ref = attr(submodel, "modelRef").unwrap_or("");
                let target = if let Some((context, _)) = context {
                    context.model(entry, model_ref)
                } else {
                    definitions.get(model_ref).map(|model| (entry, *model))
                };
                let Some((target_entry, target)) = target else {
                    return Err(format!(
                        "The submodel '{sid}' references unknown model '{model_ref}'."
                    ));
                };
                pending.push((
                    target,
                    target_entry,
                    format!("{prefix}{sid}__"),
                    Some((i, submodel)),
                    ancestry.clone(),
                ));
            }
        }
        for i in 0..this.instances.len() {
            let model = this.instances[i].model;
            let unit_ptrs: HashSet<_> = model
                .unit_definitions
                .iter()
                .map(|u| std::ptr::from_ref(&u.element) as usize)
                .collect();
            let local_ptrs: HashSet<_> = model
                .reactions
                .iter()
                .filter_map(|r| r.kinetic_law.as_ref())
                .flat_map(|l| &l.local_parameters)
                .map(|p| std::ptr::from_ref(&p.element) as usize)
                .collect();
            for element in core(model) {
                let target = Target {
                    instance: i,
                    element,
                    child: None,
                };
                if let Some(id) = element.meta_id.as_deref() {
                    this.instances[i].meta.entry(id).or_insert(target);
                }
                if let Some(id) = element.id.as_deref() {
                    if unit_ptrs.contains(&(std::ptr::from_ref(element) as usize)) {
                        this.instances[i].units.insert(id, target);
                    } else if !local_ptrs.contains(&(std::ptr::from_ref(element) as usize)) {
                        this.instances[i].sids.entry(id).or_insert(target);
                    }
                }
                for list in &element.lists {
                    if let Some(meta) = list.element.meta_id.as_deref() {
                        this.instances[i].meta.entry(meta).or_insert(Target {
                            instance: i,
                            element: &list.element,
                            child: None,
                        });
                    }
                }
            }
            for submodel in children(&model.element, "comp", "listOfSubmodels") {
                let sid = id(&submodel.element);
                let target = Target {
                    instance: i,
                    element: &submodel.element,
                    child: this.instances[i].children.get(sid).copied(),
                };
                this.instances[i].sids.insert(sid, target);
                if let Some(meta) = submodel.element.meta_id.as_deref() {
                    this.instances[i].meta.insert(meta, target);
                }
            }
        }
        Ok(this)
    }
    fn resolve(
        &self,
        reference: &Extension,
        instance: usize,
        seen: &mut HashSet<(usize, usize)>,
    ) -> Option<Target<'a>> {
        if seen.len() > 256 {
            return None;
        }
        let index = &self.instances[instance];
        let target = if let Some(port) = attr(reference, "portRef") {
            let port = *index.ports.get(port)?;
            let key = (instance, std::ptr::from_ref(port) as usize);
            if !seen.insert(key) {
                return None;
            }
            self.resolve(port, instance, seen)?
        } else if let Some(id) = attr(reference, "idRef") {
            *index.sids.get(id)?
        } else if let Some(id) = attr(reference, "unitRef") {
            *index.units.get(id)?
        } else {
            let id = attr(reference, "metaIdRef")?;
            *index.meta.get(id)?
        };
        if let Some(nested) = reference.children.iter().find(|e| e.name == "sBaseRef") {
            self.resolve(nested, target.child?, seen)
        } else {
            Some(target)
        }
    }
    fn aliases(&mut self) -> Result<(), String> {
        for i in 0..self.instances.len() {
            for element in core(self.instances[i].model) {
                let owner = Target {
                    instance: i,
                    element,
                    child: None,
                };
                if let Some(reference) = ext(element, "replacedBy") {
                    let child = attr(reference, "submodelRef")
                        .and_then(|s| self.instances[i].children.get(s))
                        .copied()
                        .ok_or_else(|| "A replacement references no submodel.".to_owned())?;
                    let target = self
                        .resolve(reference, child, &mut HashSet::new())
                        .ok_or_else(|| "A replacedBy reference names no element.".to_owned())?;
                    let owner_name = format!("{}{}", self.instances[i].prefix, id(element));
                    self.names.insert(target.key(), owner_name);
                    self.aliases.insert(owner.key(), target);
                    self.removed.insert(owner.key());
                }
                for reference in children(element, "comp", "listOfReplacedElements") {
                    let child = attr(reference, "submodelRef")
                        .and_then(|s| self.instances[i].children.get(s))
                        .copied()
                        .ok_or_else(|| "A replacement references no submodel.".to_owned())?;
                    if attr(reference, "deletion").is_some() {
                        continue;
                    }
                    let target = self
                        .resolve(reference, child, &mut HashSet::new())
                        .ok_or_else(|| {
                            "A replacedElement reference names no element.".to_owned()
                        })?;
                    if let Some(factor) = attr(reference, "conversionFactor") {
                        self.conversion.insert(target.key(), (i, factor.to_owned()));
                    }
                    self.aliases.insert(target.key(), owner);
                    self.removed.insert(target.key());
                }
            }
            for submodel in children(&self.instances[i].model.element, "comp", "listOfSubmodels") {
                let Some(child) = self.instances[i]
                    .children
                    .get(id(&submodel.element))
                    .copied()
                else {
                    continue;
                };
                for deletion in submodel
                    .children
                    .iter()
                    .filter(|e| e.name == "listOfDeletions")
                    .flat_map(|e| &e.children)
                {
                    if let Some(target) = self.resolve(deletion, child, &mut HashSet::new()) {
                        self.removed.insert(target.key());
                    }
                }
            }
        }
        Ok(())
    }
    fn canonical(&self, mut target: Target<'a>) -> Result<Target<'a>, String> {
        let mut seen = HashSet::new();
        while let Some(next) = self.aliases.get(&target.key()) {
            if !seen.insert(target.key()) {
                return Err("The comp replacements form a cycle.".into());
            }
            target = *next;
        }
        Ok(target)
    }
    fn sid(&self, target: Target<'a>) -> Result<String, String> {
        let target = self.canonical(target)?;
        Ok(self.names.get(&target.key()).cloned().unwrap_or_else(|| {
            format!(
                "{}{}",
                self.instances[target.instance].prefix,
                id(target.element)
            )
        }))
    }
    fn lookup(&self, instance: usize, sid: &str, unit: bool) -> String {
        let index = &self.instances[instance];
        let target = if unit {
            index.units.get(sid)
        } else {
            index.sids.get(sid)
        };
        target
            .and_then(|target| self.sid(*target).ok())
            .unwrap_or_else(|| sid.to_owned())
    }
    fn base(&self, element: &mut Element, instance: usize, local: bool) {
        if let Some(id) = element.id.clone() {
            element.id = Some(if local {
                id
            } else {
                self.lookup(instance, &id, false)
            });
        }
        if let Some(meta) = element.meta_id.clone() {
            element.meta_id = Some(format!("{}{meta}", self.instances[instance].prefix));
        }
        element.xml = None;
        element.extensions.clear();
        element.lists.clear();
        element.package_attributes.clear();
    }
    fn factors(&self, mut instance: usize, attribute: &str) -> Vec<Ast> {
        let mut factors = Vec::new();
        while let Some((parent, submodel)) = self.instances[instance].parent {
            if let Some(sid) = attr(submodel, attribute) {
                factors.push(name(&self.lookup(parent, sid, false)));
            }
            instance = parent;
        }
        factors
    }
    fn math(&self, math: &mut Option<Ast>, instance: usize, locals: &HashSet<String>) {
        if let Some(math) = math {
            let mut stack = vec![(math, HashSet::new())];
            while let Some((a, bound)) = stack.pop() {
                if a.kind == AstKind::Lambda {
                    let binders = a
                        .children
                        .iter()
                        .take(a.children.len().saturating_sub(1))
                        .filter_map(|p| p.name.clone())
                        .collect::<HashSet<_>>();
                    let mut scope = bound;
                    scope.extend(binders);
                    if let Some(body) = a.children.last_mut() {
                        stack.push((body, scope));
                    }
                    continue;
                }
                if a.kind == AstKind::NameTime {
                    let factors = self.factors(instance, "timeConversionFactor");
                    if !factors.is_empty() {
                        *a = node(AstKind::Divide, vec![a.clone(), product(factors)]);
                        continue;
                    }
                }
                if matches!(a.kind, AstKind::Name | AstKind::Function)
                    && let Some(sid) = a.name.clone()
                    && !bound.contains(&sid)
                    && !locals.contains(&sid)
                {
                    let factor = if a.kind == AstKind::Name {
                        self.instances[instance]
                            .sids
                            .get(sid.as_str())
                            .and_then(|target| self.conversion.get(&target.key()))
                    } else {
                        None
                    };
                    let renamed = self.lookup(instance, &sid, false);
                    if let Some((parent, factor)) = factor {
                        *a = node(
                            AstKind::Divide,
                            vec![name(&renamed), name(&self.lookup(*parent, factor, false))],
                        );
                        continue;
                    }
                    a.name = Some(renamed);
                }
                if let Some(units) = &a.units {
                    a.units = Some(self.lookup(instance, units, true));
                }
                stack.extend(a.children.iter_mut().map(|c| (c, bound.clone())));
            }
        }
    }
    fn converted_assignment(&self, math: &mut Option<Ast>, instance: usize, variable: &str) {
        let factor = self.instances[instance]
            .sids
            .get(variable)
            .and_then(|target| self.conversion.get(&target.key()));
        if let Some((parent, factor)) = factor
            && let Some(rhs) = math.take()
        {
            *math = Some(node(
                AstKind::Times,
                vec![rhs, name(&self.lookup(*parent, factor, false))],
            ));
        }
    }
    fn keep(&self, instance: usize, element: &Element) -> bool {
        !self
            .removed
            .contains(&(instance, std::ptr::from_ref(element) as usize))
    }
    fn model(&self, instance: usize) -> Model {
        let source = self.instances[instance].model;
        let mut out = source.clone();
        let none = HashSet::new();
        macro_rules! filtered {
            ($field:ident) => {
                out.$field = source
                    .$field
                    .iter()
                    .filter(|x| self.keep(instance, &x.element))
                    .cloned()
                    .collect();
            };
        }
        filtered!(function_definitions);
        filtered!(unit_definitions);
        filtered!(compartments);
        filtered!(species);
        filtered!(parameters);
        filtered!(initial_assignments);
        filtered!(rules);
        filtered!(constraints);
        filtered!(reactions);
        filtered!(events);
        for f in &mut out.function_definitions {
            self.base(&mut f.element, instance, false);
            self.math(&mut f.math, instance, &none);
        }
        for u in &mut out.unit_definitions {
            let original = u.element.clone();
            u.element.id = u
                .element
                .id
                .as_deref()
                .map(|id| self.lookup(instance, id, true));
            self.base(&mut u.element, instance, true);
            u.element.id = original
                .id
                .as_deref()
                .map(|id| self.lookup(instance, id, true));
        }
        for c in &mut out.compartments {
            self.base(&mut c.element, instance, false);
            c.units = c.units.as_deref().map(|u| self.lookup(instance, u, true));
        }
        for s in &mut out.species {
            self.base(&mut s.element, instance, false);
            s.compartment = self.lookup(instance, &s.compartment, false);
            s.substance_units = s
                .substance_units
                .as_deref()
                .map(|u| self.lookup(instance, u, true));
            s.conversion_factor = s
                .conversion_factor
                .as_deref()
                .map(|f| self.lookup(instance, f, false));
        }
        for p in &mut out.parameters {
            self.base(&mut p.element, instance, false);
            p.units = p.units.as_deref().map(|u| self.lookup(instance, u, true));
        }
        for a in &mut out.initial_assignments {
            self.base(&mut a.element, instance, false);
            self.math(&mut a.math, instance, &none);
            self.converted_assignment(&mut a.math, instance, &a.symbol);
            a.symbol = self.lookup(instance, &a.symbol, false);
        }
        for r in &mut out.rules {
            self.base(&mut r.element, instance, false);
            self.math(&mut r.math, instance, &none);
            if let Some(variable) = &r.variable {
                self.converted_assignment(&mut r.math, instance, variable);
            }
            r.variable = r
                .variable
                .as_deref()
                .map(|s| self.lookup(instance, s, false));
            let time = self.factors(instance, "timeConversionFactor");
            if r.kind == RuleKind::Rate
                && !time.is_empty()
                && let Some(math) = r.math.take()
            {
                r.math = Some(node(AstKind::Divide, vec![math, product(time)]));
            }
        }
        for c in &mut out.constraints {
            self.base(&mut c.element, instance, false);
            self.math(&mut c.math, instance, &none);
        }
        for (r, original) in out.reactions.iter_mut().zip(
            source
                .reactions
                .iter()
                .filter(|r| self.keep(instance, &r.element)),
        ) {
            self.base(&mut r.element, instance, false);
            r.compartment = r
                .compartment
                .as_deref()
                .map(|c| self.lookup(instance, c, false));
            r.reactants = original
                .reactants
                .iter()
                .filter(|x| self.keep(instance, &x.element))
                .cloned()
                .collect();
            r.products = original
                .products
                .iter()
                .filter(|x| self.keep(instance, &x.element))
                .cloned()
                .collect();
            r.modifiers = original
                .modifiers
                .iter()
                .filter(|x| self.keep(instance, &x.element))
                .cloned()
                .collect();
            for reference in r.reactants.iter_mut().chain(&mut r.products) {
                self.base(&mut reference.element, instance, false);
                reference.species = self.lookup(instance, &reference.species, false);
                self.math(&mut reference.stoichiometry_math_ast, instance, &none);
            }
            for modifier in &mut r.modifiers {
                self.base(&mut modifier.element, instance, false);
                modifier.species = self.lookup(instance, &modifier.species, false);
            }
            if let Some(law) = &original.kinetic_law {
                if !self.keep(instance, &law.element) {
                    r.kinetic_law = None;
                } else if let Some(output) = &mut r.kinetic_law {
                    self.base(&mut output.element, instance, false);
                    output.local_parameters = law
                        .local_parameters
                        .iter()
                        .filter(|p| self.keep(instance, &p.element))
                        .cloned()
                        .collect();
                    let locals = output
                        .local_parameters
                        .iter()
                        .filter_map(|p| p.element.id.clone())
                        .collect();
                    self.math(&mut output.math, instance, &locals);
                    let time = self.factors(instance, "timeConversionFactor");
                    let extent = self.factors(instance, "extentConversionFactor");
                    if (!time.is_empty() || !extent.is_empty())
                        && let Some(math) = output.math.take()
                    {
                        let factor = if time.is_empty() {
                            product(extent)
                        } else {
                            node(
                                AstKind::Divide,
                                vec![
                                    if extent.is_empty() {
                                        number(1.0)
                                    } else {
                                        product(extent)
                                    },
                                    product(time),
                                ],
                            )
                        };
                        output.math = Some(node(AstKind::Times, vec![factor, math]));
                    }
                }
            }
        }
        for e in &mut out.events {
            self.base(&mut e.element, instance, false);
            if let Some(t) = &mut e.trigger {
                self.math(&mut t.math, instance, &none);
            }
            if let Some(p) = &mut e.priority {
                self.math(&mut p.math, instance, &none);
            }
            if let Some(d) = &mut e.delay {
                self.math(&mut d.math, instance, &none);
                let time = self.factors(instance, "timeConversionFactor");
                if !time.is_empty()
                    && let Some(math) = d.math.take()
                {
                    d.math = Some(node(AstKind::Times, vec![product(time), math]));
                }
            }
            for a in &mut e.event_assignments {
                self.math(&mut a.math, instance, &none);
                self.converted_assignment(&mut a.math, instance, &a.variable);
                a.variable = self.lookup(instance, &a.variable, false);
            }
        }
        out
    }
}

pub fn flatten<'a>(
    document: &'a Document,
    context: Option<(&dyn ModelResolver<'a>, usize)>,
) -> Result<Model, String> {
    if let Some(error) = external_error(document, context) {
        return Err(error);
    }
    let mut expanded = Flatten::new(document, context)?;
    expanded.aliases()?;
    let mut models = (0..expanded.instances.len()).map(|i| expanded.model(i));
    let Some(mut model) = models.next() else {
        return Err("The SBML document has no model.".into());
    };
    for child in models {
        model
            .function_definitions
            .extend(child.function_definitions);
        model.unit_definitions.extend(child.unit_definitions);
        model.compartments.extend(child.compartments);
        model.species.extend(child.species);
        model.parameters.extend(child.parameters);
        model.initial_assignments.extend(child.initial_assignments);
        model.rules.extend(child.rules);
        model.constraints.extend(child.constraints);
        model.reactions.extend(child.reactions);
        model.events.extend(child.events);
    }
    Ok(model)
}
