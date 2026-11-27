//! Human-readable document projections of the same normalized equation plan.
use super::*;
#[path = "text_tables.rs"]
mod text_tables;

struct DocumentContext<'a> {
    fmt: &'a str,
    dialect: Dialect,
    analysis: &'a Analysis<'a>,
    symbols: HashMap<String, String>,
}
impl DocumentContext<'_> {
    fn text(&self, value: &str) -> String {
        let value = metadata::single_line(value);
        let chars = value.chars().collect::<Vec<_>>();
        let digits = chars.iter().take_while(|c| c.is_ascii_digit()).count();
        let mut out = String::new();
        for (i, c) in chars.iter().copied().enumerate() {
            if self.fmt == "latex" {
                if let Some(s) = text_tables::tex(c) {
                    out.push_str(s);
                } else {
                    out.push(c);
                }
                continue;
            }
            if self.fmt == "markdown" && matches!(c, '&' | '<' | '>') {
                out.push_str(match c {
                    '&' => "&amp;",
                    '<' => "&lt;",
                    _ => "&gt;",
                });
                continue;
            }
            let special = if self.fmt == "markdown" {
                matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '|' | '~' | '$')
                    || i == 0 && matches!(c, '#' | '+' | '-')
                    || i == digits && digits > 0 && matches!(c, '.' | ')')
            } else {
                matches!(
                    c,
                    '\\' | '#' | '$' | '*' | '_' | '@' | '<' | '>' | '[' | ']' | '`' | '~'
                ) || i == 0 && matches!(c, '-' | '+' | '=' | '/')
                    || c == '-' && chars.get(i + 1).is_some_and(|c| matches!(c, '-' | '?'))
                    || c == '/' && chars.get(i + 1) == Some(&'/')
                    || i == digits && digits > 0 && c == '.'
            };
            if special {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }
    fn optional_text(&self, text: Option<&str>) -> Option<String> {
        text.map(|s| self.text(s))
    }
    fn parts(&self, value: &str) -> Vec<String> {
        if value.chars().count() <= 12 {
            return vec![value.into()];
        }
        let mut out = vec![];
        let mut part = String::new();
        let chars = value.chars().collect::<Vec<_>>();
        for (i, c) in chars.iter().copied().enumerate() {
            part.push(c);
            if c == '_' && chars.get(i + 1) != Some(&'_') {
                out.push(std::mem::take(&mut part));
            }
        }
        if !part.is_empty() {
            out.push(part);
        }
        out
    }
    fn code(&self, sid: &str) -> String {
        if self.fmt == "markdown" {
            return format!("`{sid}`");
        }
        let sep = if self.fmt == "typst" {
            "#sym.zws;"
        } else {
            r"\allowbreak{}"
        };
        self.parts(sid)
            .iter()
            .map(|part| {
                if self.fmt == "typst" {
                    format!("`{part}`")
                } else {
                    format!(r"\texttt{{{}}}", self.text(part))
                }
            })
            .collect::<Vec<_>>()
            .join(sep)
    }
    fn breakable(&self, s: &str) -> String {
        if self.fmt != "latex" {
            return self.text(s);
        }
        self.parts(s)
            .iter()
            .map(|s| self.text(s))
            .collect::<Vec<_>>()
            .join(r"\allowbreak{}")
    }
    fn unit(&self, unit: Option<&str>) -> Option<String> {
        let unit = unit?;
        let exponent = regex::Regex::new(r"\^(-?[0-9.]+)").unwrap();
        let mut out = String::new();
        let mut last = 0;
        for capture in exponent.captures_iter(unit) {
            let whole = capture.get(0).unwrap();
            out.push_str(
                &unit[last..whole.start()]
                    .split('*')
                    .map(|p| self.text(p))
                    .collect::<Vec<_>>()
                    .join("·"),
            );
            let exp = &capture[1];
            let exp = format!(
                "{}{}",
                if exp.starts_with('-') {
                    if self.fmt == "latex" {
                        r"\textminus{}"
                    } else {
                        "−"
                    }
                } else {
                    ""
                },
                self.text(exp.trim_start_matches('-'))
            );
            out.push_str(&match self.fmt {
                "latex" => format!(r"\textsuperscript{{{exp}}}"),
                "typst" => format!("#super[{exp}]"),
                _ => format!("<sup>{exp}</sup>"),
            });
            last = whole.end();
        }
        out.push_str(
            &unit[last..]
                .split('*')
                .map(|p| self.text(p))
                .collect::<Vec<_>>()
                .join("·"),
        );
        if self.fmt == "latex" {
            out = regex::Regex::new(r"f([fil])")
                .unwrap()
                .replace_all(&out, r"f\kern0pt{}$1")
                .into_owned();
        } else if self.fmt == "typst" && regex::Regex::new(r"f[fil]").unwrap().is_match(&out) {
            out = format!("#text(ligatures: false)[{out}]");
        }
        Some(out)
    }
    fn row(&self, q: &Quantity) -> Value {
        let info = self
            .analysis
            .metadata
            .get(&q.sid)
            .cloned()
            .unwrap_or_default();
        json!({"symbol":self.symbols[&q.sid],"id":self.code(&q.sid),"long_id":q.sid.chars().count()>12,"name":self.optional_text(info.name.as_deref()),"value":if self.analysis.assignments.contains_key(&q.sid){None}else{q.value.map(|v|self.dialect.number(v))},"unit":self.unit(info.unit.as_deref()),"constant":q.constant})
    }
    fn side(&self, references: &[sbmlsim_document::SpeciesReference]) -> String {
        if references.is_empty() {
            return if self.fmt == "typst" {
                "emptyset"
            } else {
                r"\varnothing"
            }
            .into();
        }
        let changed = self
            .analysis
            .rates
            .keys()
            .chain(self.analysis.assignments.keys())
            .chain(self.analysis.initial.keys())
            .cloned()
            .chain(
                self.analysis
                    .model
                    .events
                    .iter()
                    .flat_map(|e| e.event_assignments.iter().map(|a| a.variable.clone())),
            )
            .collect::<HashSet<_>>();
        references
            .iter()
            .map(|r| {
                let factor =
                    if let Some(sid) = r.element.id.as_ref().filter(|id| changed.contains(*id)) {
                        Some(self.symbols[sid].clone())
                    } else {
                        let value = if r.stoichiometry_set {
                            r.stoichiometry
                        } else {
                            1.0
                        };
                        (value != 1.0).then(|| self.dialect.number(value))
                    };
                if let Some(factor) = factor {
                    format!(
                        "{factor} {} {}",
                        if self.fmt == "typst" { "thin" } else { r"\," },
                        self.symbols[&r.species]
                    )
                } else {
                    self.symbols[&r.species].clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" + ")
    }
}
fn notes(source: Option<&str>) -> Vec<String> {
    let Some(source) = source else { return vec![] };
    let Ok(document) = roxmltree::Document::parse(source) else {
        return vec![];
    };
    let blocks = [
        "address",
        "article",
        "aside",
        "blockquote",
        "body",
        "br",
        "caption",
        "dd",
        "details",
        "dialog",
        "div",
        "dl",
        "dt",
        "fieldset",
        "figcaption",
        "figure",
        "footer",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "header",
        "hr",
        "li",
        "main",
        "nav",
        "ol",
        "p",
        "pre",
        "section",
        "table",
        "tr",
        "ul",
    ];
    let before = regex::Regex::new(r"[\s\x00]*\x00\s*([).,;:!?])").unwrap();
    let after = regex::Regex::new(r"\(\s*\x00[\s\x00]*").unwrap();
    let mut paragraphs = vec![];
    let mut text = String::new();
    let flush = |text: &mut String, paragraphs: &mut Vec<String>| {
        let joined = before.replace_all(text, "$1");
        let joined = after.replace_all(&joined, "(");
        let joined = joined
            .replace('\0', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !joined.is_empty() {
            paragraphs.push(joined);
        }
        text.clear();
    };
    let mut pending = vec![(document.root_element(), false)];
    while let Some((node, done)) = pending.pop() {
        if node.is_text() {
            text.push('\0');
            text.push_str(node.text().unwrap_or(""));
            text.push('\0');
            continue;
        }
        if !node.is_element() {
            continue;
        }
        let block = blocks.contains(&node.tag_name().name());
        if done {
            if block {
                flush(&mut text, &mut paragraphs);
            } else if matches!(node.tag_name().name(), "td" | "th") {
                text.push(' ');
            }
            continue;
        }
        if block {
            flush(&mut text, &mut paragraphs);
        }
        pending.push((node, true));
        pending.extend(node.children().rev().map(|c| (c, false)));
    }
    flush(&mut text, &mut paragraphs);
    paragraphs
}

pub(super) fn build(
    document: &Document,
    analysis: &Analysis<'_>,
    prepared: &PreparedSystem,
    fmt: &str,
    options: &RenderOptions,
) -> Result<Value, String> {
    if !matches!(options.symbols.as_str(), "id" | "name") {
        return Err("Document symbols must be 'id' or 'name'.".into());
    }
    let dialect = if fmt == "typst" {
        Dialect::Typst
    } else {
        Dialect::Latex
    };
    let (symbols, _) =
        analysis.presentation_symbols(prepared, dialect, options.symbols == "name")?;
    let ctx = DocumentContext {
        fmt,
        dialect,
        analysis,
        symbols,
    };
    let system = analysis.typeset_prepared(prepared, dialect, options.symbols == "name")?;
    let model = &analysis.model;
    let sid = model.element.id.as_deref();
    let title = model
        .element
        .name
        .as_deref()
        .map(|n| ctx.text(n))
        .unwrap_or_else(|| ctx.breakable(sid.unwrap_or("Model")));
    let plain_title = model.element.name.as_deref().or(sid).unwrap_or("Model");
    let plain_title = if fmt == "latex" {
        metadata::single_line(plain_title)
            .chars()
            .map(|c| text_tables::pdf(c).map_or_else(|| c.to_string(), str::to_owned))
            .collect::<String>()
    } else {
        ctx.text(plain_title)
    };
    let compartments = analysis
        .quantities
        .values()
        .filter(|q| {
            analysis
                .metadata
                .get(&q.sid)
                .is_some_and(|m| m.kind == "compartment")
        })
        .map(|q| ctx.row(q))
        .collect::<Vec<_>>();
    let species = analysis
        .quantities
        .values()
        .filter(|q| q.species)
        .map(|q| {
            let mut row = ctx.row(q);
            row["compartment"] = json!(q.compartment.as_ref().map(|c| &ctx.symbols[c]));
            let s = model
                .species
                .iter()
                .find(|s| s.element.id.as_deref() == Some(&q.sid))
                .unwrap();
            let mut props = vec![if q.amount { "amount" } else { "concentration" }];
            if s.boundary_condition.unwrap_or(false) {
                props.push("boundary");
            }
            if q.constant {
                props.push("constant");
            }
            row["properties"] = json!(ctx.text(&props.join(", ")));
            row
        })
        .collect::<Vec<_>>();
    let parameters = analysis
        .quantities
        .values()
        .filter(|q| {
            analysis
                .metadata
                .get(&q.sid)
                .is_some_and(|m| matches!(m.kind, "parameter" | "species_reference"))
        })
        .map(|q| ctx.row(q))
        .collect::<Vec<_>>();
    let functions=model.function_definitions.iter().filter(|f|f.math.is_some()).zip(system["functions"].as_array().unwrap()).map(|(f,row)|json!({"id":ctx.code(&id(&f.element)),"name":ctx.optional_text(f.element.name.as_deref()),"lhs":row["lhs"],"rhs":row["lines"][0]})).collect::<Vec<_>>();
    let reactions=analysis.reactions.iter().zip(&model.reactions).map(|(r,original)|{
        let arrow=match(fmt,original.reversible.unwrap_or(true)){("typst",true)=>"harpoons.rtlb",("typst",false)=>"-->",(_,true)=>r"\rightleftharpoons",(_,false)=>r"\longrightarrow"};
        let locals=analysis.quantities.values().filter(|q|analysis.metadata.get(&q.sid).is_some_and(|m|m.source.len()==2&&m.source[0]==r.sid)).map(|q|ctx.symbols[&q.sid].clone()).collect::<Vec<_>>().join(", ");
        let modifiers=original.modifiers.iter().map(|m|ctx.symbols[&m.species].clone()).collect::<Vec<_>>().join(", ");
        Ok(json!({"symbol":ctx.symbols[&r.sid],"id":ctx.code(&r.sid),"long_id":r.sid.chars().count()>12,"name":ctx.optional_text(original.element.name.as_deref()),"equation":format!("{} {arrow} {}",ctx.side(&original.reactants),ctx.side(&original.products)),"modifiers":(!modifiers.is_empty()).then_some(modifiers),"local_parameters":(!locals.is_empty()).then_some(locals),"lines":dialect.lines(&r.rate,&ctx.symbols)?}))
    }).collect::<Result<Vec<_>,String>>()?;
    let events=prepared.events.iter().zip(system["events"].as_array().unwrap()).map(|(e,row)|{let raw=&model.events[e.index];json!({"id":ctx.code(&e.id),"name":ctx.optional_text(raw.element.name.as_deref()),"trigger":row["trigger"],"delay":row["delay"],"priority":row["priority"],"initial_value":row["initialValue"],"persistent":row["persistent"],"use_trigger_values":row["useValuesFromTriggerTime"],"assignments":row["assignments"].as_array().unwrap().iter().zip(analysis.event_assignments(e)).map(|(a,plan)|{let compartment=plan.divisor.as_ref().map(|cid|ctx.symbols[cid].clone());json!({"lhs":a["lhs"],"rhs":a["lines"][0],"conversion":compartment.as_ref().map(|_|"resized"),"compartment":compartment,"new":compartment.as_ref().map(|v|dialect.new_volume(v))})}).collect::<Vec<_>>()})}).collect::<Vec<_>>();
    let unsupported = analysis
        .unsupported
        .iter()
        .map(|(kind, sid)| {
            if !sid
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '-'))
            {
                return Err(format!("Invalid unsupported element identifier '{sid}'"));
            }
            Ok(json!({"construct":ctx.text(kind),"label":ctx.code(sid),"element":sid}))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let units = &model.units;
    let units = [
        ("time", &units.time),
        ("substance", &units.substance),
        ("extent", &units.extent),
        ("volume", &units.volume),
        ("area", &units.area),
        ("length", &units.length),
    ]
    .iter()
    .filter_map(|(kind, id)| {
        metadata::unit(id.as_deref(), model).map(|u| json!({"kind":kind,"unit":ctx.unit(Some(&u))}))
    })
    .collect::<Vec<_>>();
    Ok(
        json!({"model":{"title":title,"plain_title":plain_title,"id":sid.map(|s|ctx.code(s)),"level":document.level,"version":document.version,"source":options.source.as_deref().map(|s|ctx.breakable(s)),"generator":env!("CARGO_PKG_VERSION"),"notes":notes(model.element.notes.as_deref()).iter().map(|n|ctx.text(n)).collect::<Vec<_>>()},"units":units,"compartments":compartments,"species":species,"parameters":parameters,"functions":functions,"initial":system["initial"],"assignments":system["assignments"],"reactions":reactions,"odes":system["odes"],"events":events,"unsupported":unsupported,"dilution":analysis.odes.iter().any(|o|o.size_rate.is_some()),"options":{"standalone":options.standalone,"symbols":options.symbols}}),
    )
}
