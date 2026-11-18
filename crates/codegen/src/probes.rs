//! Delay query clocks that can influence a derivative, residual, or event trigger.
//! Their crossings of stored jumps become ordinary solver discontinuity roots.
use sbmlsim_ir::{CompileError, Expr, Model, Op, Symbol, optimize::tree_key};
use std::collections::{HashMap, HashSet};

pub(crate) struct HistoryPlan {
    pub sites: Vec<Expr>,
    pub queries: Vec<Expr>,
    /// The largest age any history read of this model can reach, `None` when a lag is not a
    /// finite nonnegative constant. The engine keeps its history only this far back.
    pub max_delay: Option<f64>,
}

impl HistoryPlan {
    pub fn build(model: &Model, mut roots: Vec<Expr>) -> Result<Self, CompileError> {
        let mut sites = crate::wasm::delay_sites(model, &roots);
        if sites.is_empty() {
            return Ok(Self {
                sites,
                queries: Vec::new(),
                max_delay: None,
            });
        }
        let max_delay = max_delay(model, &roots);
        let mut rules = HashMap::new();
        for a in &model.assignment_rules {
            let queries = collect(&a.expr, &rules)?;
            if !queries.is_empty() {
                rules.insert(a.target, queries);
            }
        }
        let mut queries = Vec::new();
        for e in model
            .rates
            .iter()
            .chain(model.events.iter().map(|e| &e.trigger))
        {
            queries.extend(collect(e, &rules)?);
        }
        deduplicate(&mut queries)?;
        let slots = sites.len().max(queries.len());
        if !queries.is_empty() {
            roots.extend(queries.iter().cloned());
            sites = crate::wasm::delay_sites(model, &roots);
        }
        // The runtime also uses the slot count to bound the scratch needed for nested
        // queries in selections and event assignments, which need no propagation root.
        queries.resize_with(slots, || Expr::Number(f64::NAN));
        Ok(Self {
            sites,
            queries,
            max_delay,
        })
    }
}

/// The largest age `e` can read, the longest chain of nested constant lags; `None` when a lag
/// is not a finite nonnegative constant, which leaves the reach of the expression unbounded.
fn reach(e: &Expr) -> Option<f64> {
    match e {
        Expr::Apply {
            op: Op::Delay,
            args,
        } => match &args[1] {
            Expr::Number(lag) if lag.is_finite() && *lag >= 0.0 => Some(lag + reach(&args[0])?),
            _ => None,
        },
        Expr::Apply { args, .. } => args.iter().try_fold(0.0f64, |a, x| Some(a.max(reach(x)?))),
        Expr::Piecewise { pieces, otherwise } => {
            let mut most = 0.0f64;
            for (value, condition) in pieces {
                most = most.max(reach(value)?).max(reach(condition)?);
            }
            if let Some(value) = otherwise {
                most = most.max(reach(value)?);
            }
            Some(most)
        }
        _ => Some(0.0),
    }
}

/// The reach of every expression of the model which can read its history.
fn max_delay(model: &Model, roots: &[Expr]) -> Option<f64> {
    let mut most = 0.0f64;
    let mut take = |e: &Expr| -> Option<()> {
        most = most.max(reach(e)?);
        Some(())
    };
    for a in model.initial.iter().chain(&model.assignment_rules) {
        take(&a.expr)?;
    }
    for e in model.rates.iter().chain(roots) {
        take(e)?;
    }
    for s in &model.selections {
        take(&s.expr)?;
    }
    for e in &model.events {
        take(&e.trigger)?;
        for a in &e.assignments {
            take(&a.value)?;
        }
        for x in e.delay.iter().chain(&e.priority) {
            take(x)?;
        }
    }
    Some(most)
}

fn deduplicate(queries: &mut Vec<Expr>) -> Result<(), CompileError> {
    let mut keys = HashSet::new();
    let mut size = 0;
    for query in queries.iter() {
        if query.depth() > 400 {
            return Err(CompileError::Invalid(
                "delayed root probe exceeds the expression depth budget".into(),
            ));
        }
    }
    queries.retain(|query| {
        let key = tree_key(query);
        if keys.insert(key.clone()) {
            size += key.len();
            true
        } else {
            false
        }
    });
    if size > 4_000_000 {
        return Err(CompileError::Invalid(
            "delayed root probes exceed the expression size budget".into(),
        ));
    }
    Ok(())
}

fn collect(e: &Expr, rules: &HashMap<Symbol, Vec<Expr>>) -> Result<Vec<Expr>, CompileError> {
    let mut result = Vec::new();
    match e {
        Expr::Symbol(s) => return Ok(rules.get(s).cloned().unwrap_or_default()),
        Expr::Apply {
            op: Op::Delay,
            args,
        } => {
            let lag = &args[1];
            result.extend(collect(lag, rules)?);
            if !matches!(lag,Expr::Number(x) if *x==0.) {
                result.push(Expr::apply(Op::HistoryQuery, vec![lag.clone()]));
            }
            for nested in collect(&args[0], rules)? {
                result.push(Expr::apply(Op::Delay, vec![nested, lag.clone()]));
            }
        }
        Expr::Apply { args, .. } => {
            for a in args {
                result.extend(collect(a, rules)?);
            }
        }
        Expr::Piecewise { pieces, otherwise } => {
            let mut inactive = Vec::new();
            for (value, condition) in pieces {
                for query in collect(condition, rules)? {
                    result.push(guarded(&inactive, None, query));
                }
                for query in collect(value, rules)? {
                    result.push(guarded(&inactive, Some(condition), query));
                }
                inactive.push(condition);
            }
            if let Some(value) = otherwise {
                for query in collect(value, rules)? {
                    result.push(guarded(&inactive, None, query));
                }
            }
        }
        _ => {}
    }
    deduplicate(&mut result)?;
    Ok(result)
}

fn guarded(inactive: &[&Expr], active: Option<&Expr>, query: Expr) -> Expr {
    if inactive.is_empty() && active.is_none() {
        return query;
    }
    let mut pieces: Vec<_> = inactive
        .iter()
        .map(|c| (Expr::Number(f64::NAN), (*c).clone()))
        .collect();
    let other = if let Some(c) = active {
        pieces.push((query, c.clone()));
        Expr::Number(f64::NAN)
    } else {
        query
    };
    Expr::Piecewise {
        pieces,
        otherwise: Some(Box::new(other)),
    }
}
