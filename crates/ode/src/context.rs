//! Target-independent equation plan projected to the executable template contract.
use super::*;
use crate::code::{self, Target};

fn comment(sid: &str, info: &metadata::Symbol) -> String {
    let mut parts = Vec::new();
    if let Some(name) = &info.name
        && name != sid
    {
        parts.push(name.clone());
    }
    if let Some(unit) = &info.unit {
        parts.push(format!("[{unit}]"));
    }
    parts.join(" ")
}
fn role(role: Role) -> &'static str {
    match role {
        Role::Constant => "constant",
        Role::Assigned => "assigned",
        Role::State => "state",
    }
}
fn uses_time(ast: &Ast) -> bool {
    let mut todo = vec![ast];
    while let Some(a) = todo.pop() {
        if a.kind == K::NameTime {
            return true;
        }
        todo.extend(&a.children);
    }
    false
}
fn needed(seeds: impl IntoIterator<Item = String>, maths: &HashMap<&str, &Ast>) -> HashSet<String> {
    let mut used = HashSet::new();
    let mut pending = seeds.into_iter().collect::<Vec<_>>();
    while let Some(sid) = pending.pop() {
        if used.insert(sid.clone())
            && let Some(a) = maths.get(sid.as_str())
        {
            pending.extend(names(a));
        }
    }
    used
}

/// An event assignment separates trigger-time values from execution-time size conversion.
pub(super) struct EventAssignment {
    pub sid: String,
    pub math: Option<Ast>,
    pub scale: Option<Ast>,
    pub divisor: Option<String>,
}
impl Analysis<'_> {
    pub(super) fn event_assignments(&self, prepared: &PreparedEvent) -> Vec<EventAssignment> {
        let assigned = &prepared.assigned;
        let mut resized = HashSet::new();
        for c in &self.model.compartments {
            let cid = id(&c.element);
            let mut pending = vec![cid.clone()];
            let mut seen = HashSet::new();
            while let Some(sid) = pending.pop() {
                if assigned.contains_key(&sid) {
                    resized.insert(cid.clone());
                    break;
                }
                if seen.insert(sid.clone())
                    && let Some(a) = self.assignments.get(&sid).or_else(|| {
                        self.reactions
                            .iter()
                            .find(|r| r.sid == sid)
                            .map(|r| &r.rate)
                    })
                {
                    pending.extend(names(a));
                }
            }
        }
        let mut plain = vec![];
        let mut divided = vec![];
        for (sid, math) in assigned {
            let q = &self.quantities[sid];
            if q.species && !q.amount && q.compartment.as_ref().is_some_and(|c| resized.contains(c))
            {
                let cid = q.compartment.as_ref().unwrap();
                divided.push(EventAssignment {
                    sid: sid.clone(),
                    math: Some(math.clone()),
                    scale: Some(name(cid)),
                    divisor: Some(cid.clone()),
                });
            } else {
                plain.push(EventAssignment {
                    sid: sid.clone(),
                    math: Some(math.clone()),
                    scale: None,
                    divisor: None,
                });
            }
        }
        for (sid, q) in &self.quantities {
            if q.species
                && !q.amount
                && q.role != Role::Assigned
                && !assigned.contains_key(sid)
                && q.compartment.as_ref().is_some_and(|c| resized.contains(c))
            {
                let cid = q.compartment.as_ref().unwrap();
                divided.push(EventAssignment {
                    sid: sid.clone(),
                    math: None,
                    scale: Some(product(vec![name(sid), name(cid)])),
                    divisor: Some(cid.clone()),
                });
            }
        }
        plain.extend(divided);
        plain
    }
}
fn root(ast: &Ast) -> Result<Ast, String> {
    let fold = |kind: K, args: Vec<Ast>| {
        if args.len() == 1 {
            args.into_iter().next().unwrap()
        } else {
            node(kind, args)
        }
    };
    Ok(match ast.kind {
        K::ConstantTrue => number(1.0),
        K::ConstantFalse => number(-1.0),
        K::Integer | K::Real | K::RealE | K::Rational => {
            number(if value(ast) == Some(0.0) { -1.0 } else { 1.0 })
        }
        K::RelationalGt | K::RelationalGeq | K::RelationalLt | K::RelationalLeq => fold(
            K::FunctionMin,
            ast.children
                .windows(2)
                .map(|pair| {
                    if matches!(ast.kind, K::RelationalGt | K::RelationalGeq) {
                        node(K::Minus, pair.to_vec())
                    } else {
                        node(K::Minus, vec![pair[1].clone(), pair[0].clone()])
                    }
                })
                .collect(),
        ),
        K::LogicalNot => negate(root(ast.children.first().ok_or("not without operand")?)?),
        K::LogicalAnd | K::LogicalOr => {
            let args = ast
                .children
                .iter()
                .map(root)
                .collect::<Result<Vec<_>, _>>()?;
            if args.is_empty() {
                number(if ast.kind == K::LogicalAnd { 1.0 } else { -1.0 })
            } else {
                fold(
                    if ast.kind == K::LogicalAnd {
                        K::FunctionMin
                    } else {
                        K::FunctionMax
                    },
                    args,
                )
            }
        }
        K::LogicalImplies => node(
            K::FunctionMax,
            vec![negate(root(&ast.children[0])?), root(&ast.children[1])?],
        ),
        K::LogicalXor => {
            let mut args = ast.children.iter().map(root);
            let Some(first) = args.next() else {
                return Ok(number(-1.0));
            };
            let mut total = first?;
            for a in args {
                let a = a?;
                total = node(
                    K::FunctionMax,
                    vec![
                        node(K::FunctionMin, vec![total.clone(), negate(a.clone())]),
                        node(K::FunctionMin, vec![negate(total), a]),
                    ],
                );
            }
            total
        }
        _ => return Err("The event trigger has no continuous root function.".into()),
    })
}

struct Context<'a> {
    analysis: &'a Analysis<'a>,
    prepared: &'a PreparedSystem,
    target: Target,
    codes: HashMap<String, String>,
    taken: HashSet<String>,
}
impl Context<'_> {
    fn expr(&self, math: &Ast, condition: bool) -> Result<String, String> {
        code::print(math, self.target, &self.codes, condition)
    }
    fn optional(&self, math: Option<&Ast>) -> Result<Option<String>, String> {
        math.map(|m| self.expr(m, false)).transpose()
    }
    fn unique(&mut self, mut sid: String) -> String {
        while !self.taken.insert(sid.clone()) {
            sid.push('_');
        }
        sid
    }
    fn info(&self, sid: &str) -> metadata::Symbol {
        self.analysis.metadata.get(sid).cloned().unwrap_or_default()
    }
    fn variable(&self, sid: &str, index: usize) -> Value {
        let info = self.info(sid);
        let q = self.analysis.quantities.get(sid);
        json!({"id":sid,"name":info.name,"unit":info.unit,"code":self.codes[sid],"index":index+self.target.index(),"value":q.and_then(|q|q.value).map(|v|self.target.number(v)),"comment":comment(sid,&info),"kind":info.kind})
    }
    fn assignment(&self, a: &Assignment) -> Result<Value, String> {
        Ok(
            json!({"id":a.sid,"code":self.codes[&a.sid],"expr":self.expr(&a.math,false)?,"comment":comment(&a.sid,&self.info(&a.sid)),"origin":a.origin,"role":self.analysis.quantities.get(&a.sid).map_or("assigned",|q|role(q.role))}),
        )
    }
    fn scope(
        &self,
        maths: &[&Ast],
        states: &[Value],
        constants: &[Value],
        assignments: &[Value],
    ) -> Value {
        let rules = self
            .prepared
            .assignments
            .iter()
            .map(|a| (a.sid.as_str(), &a.math))
            .collect();
        let used = needed(maths.iter().flat_map(|a| names(a)), &rules);
        let time = maths.iter().any(|a| uses_time(a))
            || self
                .prepared
                .assignments
                .iter()
                .any(|a| used.contains(&a.sid) && uses_time(&a.math));
        scope(&used, time, states, constants, assignments, &HashMap::new())
    }
    fn function(
        &self,
        function: &sbmlsim_document::FunctionDefinition,
    ) -> Result<Option<Value>, String> {
        let Some(lambda) = &function.math else {
            return Ok(None);
        };
        let Some(body) = lambda.children.last() else {
            return Ok(None);
        };
        let args = lambda.children[..lambda.children.len() - 1]
            .iter()
            .map(|a| a.name.as_deref().unwrap_or(""))
            .collect::<Vec<_>>();
        let mut symbols = code::names(args.iter().copied(), self.target)?;
        let mut taken = self
            .analysis
            .model
            .function_definitions
            .iter()
            .filter_map(|f| self.codes.get(&id(&f.element)))
            .cloned()
            .collect::<HashSet<_>>();
        for arg in &args {
            let code = symbols.get_mut(*arg).unwrap();
            while !taken.insert(code.clone()) {
                code.push('_');
            }
        }
        let arguments = args.iter().map(|a| symbols[*a].clone()).collect::<Vec<_>>();
        for f in &self.analysis.model.function_definitions {
            let sid = id(&f.element);
            if let Some(code) = self.codes.get(&sid) {
                symbols.entry(sid).or_insert_with(|| code.clone());
            }
        }
        let sid = id(&function.element);
        Ok(Some(
            json!({"id":sid,"code":self.codes[&sid],"name":self.info(&sid).name,"arguments":arguments,"body":code::print(body,self.target,&symbols,false)?}),
        ))
    }
    fn event(
        &mut self,
        event: &PreparedEvent,
        states: &[Value],
        constants: &[Value],
        assignments: &[Value],
    ) -> Result<Value, String> {
        let mut rows = vec![];
        let mut sizes = Vec::<String>::new();
        let plan = self.analysis.event_assignments(event);
        for a in &plan {
            let variable = states
                .iter()
                .chain(constants)
                .find(|v| v["id"] == a.sid)
                .ok_or_else(|| format!("No event vector position for '{}'", a.sid))?;
            let divisor_index = a.divisor.as_ref().map(|cid| {
                if !sizes.contains(cid) {
                    sizes.push(cid.clone());
                }
                sizes.iter().position(|c| c == cid).unwrap()
            });
            rows.push(json!({"id":a.sid,"code":self.codes[&a.sid],"kind":if states.iter().any(|s|s["id"]==a.sid){"state"}else{"constant"},"index":variable["index"],"comment":variable["comment"],"expr":self.optional(a.math.as_ref())?,"scale":self.optional(a.scale.as_ref())?,"divisor_index":divisor_index}));
        }
        let raw = &self.analysis.model.events[event.index];
        let code = self.codes[&event.id].clone();
        let functions = json!({"delay":event.optional[0].as_ref().map(|_|self.unique(format!("event_delay_{code}"))),"priority":event.optional[1].as_ref().map(|_|self.unique(format!("event_priority_{code}"))),"values":self.unique(format!("event_values_{code}")),"assign":self.unique(format!("event_assign_{code}")),"sizes":(!sizes.is_empty()).then(||self.unique(format!("event_sizes_{code}")))});
        let size_maths = sizes.iter().map(|cid| name(cid)).collect::<Vec<_>>();
        let scopes = json!({"delay":self.scope(&event.optional[0].iter().collect::<Vec<_>>(),states,constants,assignments),"priority":self.scope(&event.optional[1].iter().collect::<Vec<_>>(),states,constants,assignments),"values":self.scope(&plan.iter().filter_map(|a|a.math.as_ref()).collect::<Vec<_>>(),states,constants,assignments),"assign":self.scope(&plan.iter().filter_map(|a|a.scale.as_ref()).collect::<Vec<_>>(),states,constants,assignments),"sizes":self.scope(&size_maths.iter().collect::<Vec<_>>(),states,constants,assignments)});
        let trigger = self.analysis.expand_functions(&event.trigger, 0)?;
        Ok(
            json!({"id":event.id,"code":code,"name":raw.element.name.as_deref().map(metadata::single_line),"comment":comment(&event.id,&self.info(&event.id)),"trigger":self.expr(&event.trigger,true)?,"root":self.expr(&root(&trigger)?,false)?,"initial_value":raw.trigger.as_ref().and_then(|t|t.initial_value).unwrap_or(true),"persistent":raw.trigger.as_ref().and_then(|t|t.persistent).unwrap_or(true),"delay":self.optional(event.optional[0].as_ref())?,"priority":self.optional(event.optional[1].as_ref())?,"use_trigger_values":raw.use_values_from_trigger_time.unwrap_or(true),"assignments":rows,"sizes":sizes.iter().enumerate().map(|(index,cid)|json!({"id":cid,"code":self.codes[cid],"index":index})).collect::<Vec<_>>(),"functions":functions,"scopes":scopes}),
        )
    }
}
fn scope(
    used: &HashSet<String>,
    time: bool,
    states: &[Value],
    constants: &[Value],
    assignments: &[Value],
    computed: &HashMap<&str, &str>,
) -> Value {
    let evaluated = assignments
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect::<HashSet<_>>();
    let unpack = |v: &&Value| {
        v["id"]
            .as_str()
            .is_some_and(|id| used.contains(id) && !evaluated.contains(id))
    };
    json!({"states":states.iter().filter(unpack).collect::<Vec<_>>(),"constants":constants.iter().filter(unpack).collect::<Vec<_>>(),"assignments":assignments.iter().filter(|a|a["id"].as_str().is_some_and(|id|used.contains(id))).collect::<Vec<_>>(),"time":time,"computed":constants.iter().filter_map(|c|computed.get(c["id"].as_str()?).map(|origin|{let mut c=c.clone();c["origin"]=json!(origin);c})).collect::<Vec<_>>()})
}

pub(super) fn build(
    document: &Document,
    analysis: &Analysis<'_>,
    prepared: &PreparedSystem,
    target: Target,
    simulator: bool,
    source: Option<&str>,
) -> Result<Value, String> {
    if !analysis.unsupported.is_empty() {
        return Err(format!(
            "The model uses constructs the {} code does not support: {}.",
            if target == Target::Diffrax {
                "diffrax"
            } else {
                target.name()
            },
            analysis
                .unsupported
                .iter()
                .map(|(kind, id)| format!("{kind} '{id}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let ids = analysis
        .quantities
        .keys()
        .map(String::as_str)
        .chain(analysis.reactions.iter().map(|r| r.sid.as_str()))
        .chain(
            analysis
                .model
                .function_definitions
                .iter()
                .filter(|f| f.math.is_some())
                .filter_map(|f| f.element.id.as_deref()),
        )
        .chain(prepared.events.iter().map(|e| e.id.as_str()))
        .chain(analysis.size_rates.values().map(String::as_str))
        .collect::<Vec<_>>();
    let codes = code::names(ids, target)?;
    let taken = codes
        .values()
        .cloned()
        .chain(target.reserved().iter().map(|s| (*s).into()))
        .collect();
    let mut ctx = Context {
        analysis,
        prepared,
        target,
        codes,
        taken,
    };
    let states = analysis
        .odes
        .iter()
        .enumerate()
        .map(|(i, o)| ctx.variable(&o.sid, i))
        .collect::<Vec<_>>();
    let constants = analysis
        .quantities
        .values()
        .filter(|q| q.role == Role::Constant)
        .enumerate()
        .map(|(i, q)| ctx.variable(&q.sid, i))
        .collect::<Vec<_>>();
    let assigned = prepared
        .assignments
        .iter()
        .enumerate()
        .map(|(i, a)| ctx.variable(&a.sid, i))
        .collect::<Vec<_>>();
    let initial = prepared
        .initial
        .iter()
        .map(|a| ctx.assignment(a))
        .collect::<Result<Vec<_>, _>>()?;
    let assignments = prepared
        .assignments
        .iter()
        .map(|a| ctx.assignment(a))
        .collect::<Result<Vec<_>, _>>()?;
    let odes=analysis.odes.iter().enumerate().map(|(index,o)|Ok(json!({"id":o.sid,"code":ctx.codes[&o.sid],"index":index+target.index(),"expr":ctx.expr(prepared.state_rhs.get(&o.sid).unwrap_or(&o.rhs),false)?,"comment":comment(&o.sid,&ctx.info(&o.sid))}))).collect::<Result<Vec<_>,String>>()?;
    let functions = analysis
        .model
        .function_definitions
        .iter()
        .map(|f| ctx.function(f))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let events = prepared
        .events
        .iter()
        .map(|e| ctx.event(e, &states, &constants, &assignments))
        .collect::<Result<Vec<_>, _>>()?;
    let event_ids = prepared
        .events
        .iter()
        .flat_map(|e| analysis.event_assignments(e).into_iter().map(|a| a.sid))
        .collect::<HashSet<_>>();
    let event_constants = constants
        .iter()
        .filter(|c| event_ids.contains(c["id"].as_str().unwrap()))
        .collect::<Vec<_>>();
    let computed = prepared
        .initial
        .iter()
        .filter(|a| {
            analysis
                .quantities
                .get(&a.sid)
                .is_some_and(|q| q.role == Role::Constant)
        })
        .map(|a| (a.sid.as_str(), a.origin))
        .collect::<HashMap<_, _>>();
    let maths = prepared
        .initial
        .iter()
        .map(|a| (a.sid.as_str(), &a.math))
        .collect();
    let initial_used = needed(
        states
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_owned())
            .chain(computed.keys().map(|s| (*s).to_owned())),
        &maths,
    );
    let initial_scope = scope(
        &initial_used,
        prepared
            .initial
            .iter()
            .any(|a| initial_used.contains(&a.sid) && uses_time(&a.math)),
        &states,
        &constants,
        &initial,
        &computed,
    );
    let dxdt = ctx.scope(
        &analysis
            .odes
            .iter()
            .map(|o| prepared.state_rhs.get(&o.sid).unwrap_or(&o.rhs))
            .collect::<Vec<_>>(),
        &states,
        &constants,
        &assignments,
    );
    let y = ctx.scope(
        &prepared
            .assignments
            .iter()
            .map(|a| &a.math)
            .collect::<Vec<_>>(),
        &states,
        &constants,
        &assignments,
    );
    // Every assigned output must be evaluated, including literal and identity assignments.
    let mut y = y;
    y["assignments"] = json!(assignments);
    let triggers = ctx.scope(
        &prepared
            .events
            .iter()
            .map(|e| &e.trigger)
            .collect::<Vec<_>>(),
        &states,
        &constants,
        &assignments,
    );
    let roots = prepared
        .events
        .iter()
        .map(|e| {
            analysis
                .expand_functions(&e.trigger, 0)
                .and_then(|a| root(&a))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let root_scope = ctx.scope(
        &roots.iter().collect::<Vec<_>>(),
        &states,
        &constants,
        &assignments,
    );
    let units = &analysis.model.units;
    let model_units = json!({"time":metadata::unit(units.time.as_deref(),analysis.model),"substance":metadata::unit(units.substance.as_deref(),analysis.model),"extent":metadata::unit(units.extent.as_deref(),analysis.model),"volume":metadata::unit(units.volume.as_deref(),analysis.model),"area":metadata::unit(units.area.as_deref(),analysis.model),"length":metadata::unit(units.length.as_deref(),analysis.model)});
    let sid = analysis.model.element.id.as_deref();
    if let Some(sid) = sid {
        crate::latex::symbol(sid)?;
    }
    let module = match sid {
        None => "Model".into(),
        Some(sid) if sid.bytes().all(|b| b == b'_') => format!("Model{sid}"),
        Some(sid) => format!("{}{}", sid[..1].to_ascii_uppercase(), &sid[1..]),
    };
    let module = ctx.unique(module);
    let mut context = json!({"model":{"id":sid,"name":analysis.model.element.name.as_deref().map(metadata::single_line),"level":document.level,"version":document.version,"source":source.map(metadata::single_line),"units":model_units,"generator":env!("CARGO_PKG_VERSION"),"module":module},"states":states,"constants":constants,"assigned":assigned,"initial":initial,"assignments":assignments,"odes":odes,"functions":functions,"events":events,"event_constants":event_constants,"scopes":{"initial":initial_scope,"dxdt":dxdt,"y":y,"triggers":triggers,"roots":root_scope},"options":{"simulator":simulator}});
    let text = context.to_string();
    let modules = match target {
        Target::Python => vec!["math", "np"],
        Target::Diffrax => vec!["jax", "jnp"],
        Target::Julia => vec!["NaNMath", "SpecialFunctions"],
        Target::R => vec![],
    }
    .into_iter()
    .filter(|module| {
        if *module == "SpecialFunctions" {
            text.contains("gamma(")
        } else {
            text.contains(&format!("{module}."))
        }
    })
    .collect::<Vec<_>>();
    context["modules"] = json!(modules);
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn differences(a: &Value, b: &Value, path: &str, errors: &mut Vec<String>) {
        if errors.len() > 70 {
            return;
        }
        match (a, b) {
            (Value::Object(a), Value::Object(b)) => {
                for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                    differences(
                        a.get(key).unwrap_or(&Value::Null),
                        b.get(key).unwrap_or(&Value::Null),
                        &format!("{path}.{key}"),
                        errors,
                    );
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    errors.push(format!("{path}: lengths {} != {}", a.len(), b.len()));
                }
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    differences(a, b, &format!("{path}[{i}]"), errors);
                }
            }
            _ if a != b => errors.push(
                format!("{path}: got {a}, expected {b}")
                    .chars()
                    .take(400)
                    .collect(),
            ),
            _ => {}
        }
    }
    #[test]
    fn all_executable_contexts_preserve_the_complete_legacy_plan() {
        let mut failures = vec![];
        for label in ["demo", "repressilator", "events", "variable_compartment"] {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/ode");
            let document =
                sbmlsim_reader::read(&std::fs::read(root.join(format!("{label}.xml"))).unwrap())
                    .unwrap();
            let expected: Value = serde_json::from_str(
                &std::fs::read_to_string(root.join(format!("{label}.contexts.json"))).unwrap(),
            )
            .unwrap();
            for target in [Target::Python, Target::Diffrax, Target::Julia, Target::R] {
                let fmt = if target == Target::Diffrax {
                    "diffrax"
                } else {
                    target.name()
                };
                let links = Links::default();
                let model = normalize(&document, document.model.as_ref().unwrap());
                let mut analysis = Analysis::new(&model, &links);
                analysis.read().unwrap();
                let prepared = analysis.prepare().unwrap();
                let actual = build(&document, &analysis, &prepared, target, false, None).unwrap();
                let mut reference = expected[format!("{fmt}_false")].clone();
                let info = reference["model"].as_object_mut().unwrap();
                info.remove("sbmlode");
                info.insert("generator".into(), json!(env!("CARGO_PKG_VERSION")));
                differences(
                    &actual,
                    &reference,
                    &format!("{label}.{fmt}"),
                    &mut failures,
                );
            }
        }
        assert!(
            failures.is_empty(),
            "{} differences:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
