//! Embedded, portable templates. Rendering has no filesystem or interpreter access.
use crate::metadata::single_line;
use minijinja::{
    Environment, Error, Value,
    value::{Kwargs, Rest, ValueOrKwargs, from_args},
};
use serde_json::json;
use std::borrow::Cow;

fn plain(v: &Value) -> String {
    if v.is_none() {
        String::new()
    } else {
        single_line(&v.to_string())
    }
}
fn escaped(v: &Value, target: &str) -> String {
    let mut text = plain(v).replace('\\', "\\\\").replace('"', "\\\"");
    if target == "julia" {
        text = text.replace('$', "\\$");
    }
    if target == "r" {
        text = text
            .chars()
            .map(|c| {
                if c.is_ascii() {
                    c.to_string()
                } else {
                    format!(
                        "\\{}{{{:0width$x}}}",
                        if u32::from(c) <= 0xffff { 'u' } else { 'U' },
                        u32::from(c),
                        width = if u32::from(c) <= 0xffff { 4 } else { 8 }
                    )
                }
            })
            .collect();
    }
    text
}
fn literal(v: Value, target: &str) -> String {
    if v.is_none() {
        match target {
            "julia" => "nothing",
            "r" => "NA_character_",
            _ => "None",
        }
        .into()
    } else {
        format!("\"{}\"", escaped(&v, target))
    }
}
fn wrapped(arguments: Rest<ValueOrKwargs>) -> Result<String, Error> {
    let arguments = arguments.into_values();
    let (args, kwargs) = from_args::<(&[Value], Kwargs)>(&arguments)?;
    let (items, indent, column, rest) = from_args::<(Vec<Value>, usize, usize, Rest<Value>)>(args)?;
    let (tail, width, brackets, trailing) = from_args::<(
        Option<usize>,
        Option<usize>,
        Option<Vec<String>>,
        Option<bool>,
    )>(&rest)?;

    let tail = tail.unwrap_or(1);
    let width = width.unwrap_or(88);
    let brackets = brackets
        .or(kwargs.get("brackets")?)
        .unwrap_or_else(|| vec!["[".into(), "]".into()]);
    let trailing = trailing.or(kwargs.get("trailing")?).unwrap_or(true);
    kwargs.assert_all_used()?;
    if brackets.len() != 2 {
        return Err(Error::new(
            minijinja::ErrorKind::InvalidOperation,
            "wrapped needs two brackets",
        ));
    }
    let text = items.iter().map(Value::to_string).collect::<Vec<_>>();
    let single = format!("{}{}{}", brackets[0], text.join(", "), brackets[1]);
    if column + single.chars().count() + tail <= width {
        return Ok(single);
    }
    let inner = " ".repeat(indent + 4);
    let mut lines = vec![];
    let mut line = String::new();
    for item in text {
        let candidate = if line.is_empty() {
            format!("{item},")
        } else {
            format!("{line} {item},")
        };
        if !line.is_empty() && inner.len() + candidate.chars().count() > width {
            lines.push(line);
            line = format!("{item},");
        } else {
            line = candidate;
        }
    }
    if !trailing {
        line = line.strip_suffix(',').unwrap_or(&line).into();
    }
    lines.push(line);
    Ok(format!(
        "{}\n{}\n{}{}",
        brackets[0],
        lines
            .iter()
            .map(|line| format!("{inner}{line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        " ".repeat(indent),
        brackets[1]
    ))
}

pub(super) fn render(fmt: &str, context: serde_json::Value) -> Result<String, String> {
    render_custom(fmt, context, None)
}

pub(super) fn render_custom(
    fmt: &str,
    mut context: serde_json::Value,
    template_override: Option<&str>,
) -> Result<String, String> {
    let target = if fmt == "diffrax" { "python" } else { fmt };
    let units = context["model"]["units"]
        .as_object()
        .map(|m| {
            ["time", "substance", "extent", "volume", "area", "length"]
                .into_iter()
                .filter_map(|k| m.get(k).filter(|v| !v.is_null()).map(|v| json!([k, v])))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    context["unit_rows"] = json!(units);
    let computed = context["scopes"]["initial"]["computed"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|c| {
                    json!([
                        format!("p[{}]", c["index"]),
                        c["id"],
                        if c["origin"] == "initial_assignment" {
                            "initial assignment"
                        } else {
                            "converted with its compartment"
                        }
                    ])
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    context["computed_rows"] = json!(computed);
    let literals = context["event_constants"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|v| format!("\"{}\"", v["id"].as_str().unwrap()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    context["event_constant_literals"] = json!(literals);
    if let Some(events) = context["events"].as_array_mut() {
        for e in events {
            for flag in ["initial_value", "persistent", "use_trigger_values"] {
                e[format!("literal_{flag}")] = json!(if e[flag].as_bool().unwrap_or(false) {
                    "True"
                } else {
                    "False"
                });
            }
        }
    }
    let mut env = Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    env.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
    env.set_syntax(
        minijinja::syntax::SyntaxConfig::builder()
            .trim_blocks(true)
            .lstrip_blocks(true)
            .keep_trailing_newline(true)
            .build()
            .map_err(|e| e.to_string())?,
    );
    env.add_filter("single_line", |v: Value| plain(&v));
    for (name, target) in [
        ("docstring", "python"),
        ("julia_text", "julia"),
        ("r_text", "r"),
    ] {
        env.add_filter(name, move |v: Value| escaped(&v, target));
        env.add_function(name, move |v: Value| escaped(&v, target));
    }
    env.add_filter("python_string", |v: Value| literal(v, "python"));
    env.add_filter("julia_string", |v: Value| literal(v, "julia"));
    env.add_filter("r_string", |v: Value| literal(v, "r"));
    env.add_function("wrapped", wrapped);
    env.add_function(
        "rows",
        |items: Vec<Value>, keys: Rest<String>| -> Result<Value, Error> {
            let rows = items
                .iter()
                .map(|item| {
                    keys.iter()
                        .map(|key| item.get_item(&Value::from(key.as_str())))
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::from_iter(rows.into_iter().map(Value::from_iter)))
        },
    );
    let target = target.to_owned();
    env.add_function(
        "table",
        move |rows: Vec<Vec<Value>>, indent: Option<usize>, _escape: Option<Value>| -> String {
            if rows.is_empty() {
                return String::new();
            }
            let rows = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|v| {
                            if v.is_none() {
                                String::new()
                            } else {
                                escaped(v, &target)
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let width = rows[0].len();
            let widths = (0..width)
                .map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap())
                .collect::<Vec<_>>();
            rows.iter()
                .map(|row| {
                    format!(
                        "{}{}",
                        " ".repeat(indent.unwrap_or(0)),
                        row.iter()
                            .zip(&widths)
                            .map(|(c, w)| format!("{c}{}", " ".repeat(w - c.chars().count())))
                            .collect::<Vec<_>>()
                            .join("  ")
                    )
                    .trim_end()
                    .to_owned()
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    let (name, _, template) = TEMPLATES
        .iter()
        .find(|(name, _, _)| *name == fmt)
        .ok_or_else(|| format!("Unknown ODE format '{fmt}'"))?;
    if template_override.is_some() {
        for (_, filename, source) in TEMPLATES {
            env.add_template_owned(filename, canonical_newlines(source))
                .map_err(|e| format!("{e:#}"))?;
        }
    }
    env.add_template_owned(
        *name,
        canonical_newlines(template_override.unwrap_or(template)),
    )
    .map_err(|e| format!("{e:#}"))?;
    env.get_template(name)
        .map_err(|e| format!("{e:#}"))?
        .render(minijinja::value::Serde(&context))
        .map_err(|e| format!("{e:#}"))
}

const TEMPLATES: [(&str, &str, &str); 7] = [
    (
        "python",
        "python.py.jinja",
        include_str!("../templates/python.py.jinja"),
    ),
    (
        "diffrax",
        "diffrax.py.jinja",
        include_str!("../templates/diffrax.py.jinja"),
    ),
    (
        "julia",
        "julia.jl.jinja",
        include_str!("../templates/julia.jl.jinja"),
    ),
    ("r", "r.R.jinja", include_str!("../templates/r.R.jinja")),
    (
        "markdown",
        "markdown.md.jinja",
        include_str!("../templates/markdown.md.jinja"),
    ),
    (
        "latex",
        "latex.tex.jinja",
        include_str!("../templates/latex.tex.jinja"),
    ),
    (
        "typst",
        "typst.typ.jinja",
        include_str!("../templates/typst.typ.jinja"),
    ),
];

// MiniJinja preserves source line endings. Normalize text from Windows checkouts
// and custom templates so native and browser artifacts have identical bytes.
fn canonical_newlines(source: &str) -> Cow<'_, str> {
    if source.contains('\r') {
        Cow::Owned(source.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OdeSystem, RenderOptions};

    #[test]
    fn windows_templates_preserve_complete_generated_files() {
        let source = include_bytes!("../../../testdata/ode/events.xml");
        let system = OdeSystem::read(source).unwrap();
        let options = RenderOptions::default();
        for (format, _, template) in TEMPLATES {
            let windows = template
                .lines()
                .map(|line| format!("{line}\r\n"))
                .collect::<String>();
            assert_eq!(
                system
                    .render_template(format, &options, &windows)
                    .unwrap()
                    .content,
                system.render(format, &options).unwrap().content,
                "{format}"
            );
        }
        let rendered = system
            .render_template(
                "python",
                &options,
                "first\r\n{{ model.id }}\r\nlast\rtail\r\n",
            )
            .unwrap();
        assert_eq!(rendered.content, "first\nevents_model\nlast\ntail\n");
    }

    #[test]
    fn custom_templates_can_include_embedded_formats() {
        let system = OdeSystem::read(include_bytes!("../../../testdata/ode/events.xml")).unwrap();
        let options = RenderOptions::default();
        let rendered = system
            .render_template("python", &options, r#"{% include "python.py.jinja" %}"#)
            .unwrap();
        assert_eq!(
            rendered.content,
            system.render("python", &options).unwrap().content
        );
    }
}
