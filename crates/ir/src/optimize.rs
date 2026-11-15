//! The optimizer: smaller and faster functions of `model.wasm` with the bits of the code as
//! written.
//!
//! The emitted code of an expression is a sequence of IEEE 754 binary64 operations and calls of
//! mathlib (`sbmlsim-codegen`, the meanings of [`crate::eval`]). The optimizer changes only what
//! gives the same double for every input, NaN, the infinities and the signed zeros included (a
//! NaN as a NaN: wasm does not fix the payload of a NaN an operation gives), so a model compiled
//! with and without it gives bitwise equal results. Its passes:
//!
//! 1. **Constant folding and exact rewrites** ([`fold`]), on every expression of a function:
//!
//! | rewrite | why it is exact |
//! |---|---|
//! | an operator whose operands are all literals (numbers or booleans), e.g. `2 * 3`, `exp(1)`, `1 < 2` | it is evaluated with the operations of the emitted code ([`crate::eval`]): arithmetic in `f64`, the math functions through `sbmlsim_mathlib::f`, the libm of `mathlib.wasm`, the relations unordered, the booleans as the emitted code reads them; the double is the one the code computes |
//! | the leading literals of `+`, `*`, `max`, `min`: `1 + 2 + x` is `3 + x` | the code is a left fold, `(1 + 2) + x`, so the leading literals are the first operations of the fold, of literals; `x + 1 + 2` is `(x + 1) + 2` and stays |
//! | `x * 1`, `1 * x` (a factor of the bits of `1.0` of a product of more than one factor, anywhere in the fold) | `y * 1 = y` for every double `y` (`-0 * 1 = -0`, `inf * 1 = inf`, NaN stays NaN); a factor 1 inside the fold multiplies the partial product by 1 |
//! | `x / 1` | `y / 1 = y` for every double |
//! | `x - 0` (the bits of `+0.0`) | `y - (+0) = y` for every double, also `-0 - (+0) = -0`; `x - (-0)` is `x + 0`, `+0` for `x = -0`, and stays |
//! | `+(x)`, `*(x)`, `max(x)`, `min(x)` of one argument are `x` | the code of the operator of one argument is the code of the argument |
//! | `max(a, b, c, ...)`, `min(...)` are `max(max(a, b), c)` | the code is the left fold `a > b ? a : b`, then with `c`, which the nested form computes with the same comparisons in the same order; the prefix `max(a, b)` of the value is then the subtree the derivative of `max` reads ([`crate::derive`]) |
//! | a piece of a `piecewise` whose condition is a literal false is dropped; a literal true condition ends the pieces (its value is the `otherwise`, the value itself when it is the first piece); no piece left is the `otherwise`, NaN without | the code evaluates the conditions in their order and takes the first true one, a literal condition is the same in every evaluation, and every expression is pure |
//!
//! Nothing else: not `x + 0` (`-0 + 0 = +0`), `0 * x`, `x * 0` (NaN or the sign of 0), `x - x`,
//! `x / x` (NaN of `inf` and `0`), `-(-x)` (the negation is `0 - x`, and `0 - (0 - (-0))` is
//! `+0`), no reassociation, no distribution, and no folding across a symbol.
//!
//! 2. **Dead code elimination** ([`program`]): a function evaluates only the assignment rules
//!    (and the derivatives of a Jacobian) its outputs read, directly or through other rules
//!    (`model_rhs` the rules of the rates, `model_jac` those of the entries and their
//!    derivatives, `model_roots` those of the root functions, the functions of an event index
//!    those of any event). `model_selections` has every selection of the description as an
//!    output.
//!
//! 3. **Common subexpressions** ([`program`]): identical subtrees of the expressions of one
//!    function (equal as trees with the bits of their numbers: `0.0` and `-0.0` differ, and so
//!    do NaNs of other payloads) are computed once into a local ([`Symbol::Common`]), defined
//!    right before the first statement which needs it, its own common subexpressions first.
//!    A boolean (a relation or a boolean operator) is no common subexpression, since the locals
//!    are doubles. Every expression is pure and neither an `f64` operation nor a call of
//!    mathlib traps, so computing a subtree earlier gives its value. The `piecewise` of the code
//!    is lazy (only the conditions up to the first true one and its value are evaluated), so a
//!    subtree is shared only when one of its occurrences is evaluated in any case (in no value,
//!    `otherwise` or condition after the first of a `piecewise`): an occurrence in a lazy
//!    branch then reads the local, and no subtree is computed which the code as written does not
//!    compute. A subtree all of whose occurrences are lazy stays in its branches.
//!
//! A program of an assignment rule whose expression is a common subexpression of later
//! statements reuses the local of the rule. The programs of the functions of a model are
//! [`plan`]; codegen emits them (and the code as written without the optimizer).
//!
//! The crate does no file system or OS access.

use std::collections::{HashMap, HashSet};

use crate::derive::JacobianIr;
use crate::eval::{eval, eval_bool, is_boolean};
use crate::{Assignment, Event, Expr, Model, Op, Symbol, index};

/// A function as a list of statements.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Program {
    /// The statements in their order.
    pub statements: Vec<Statement>,
}

/// A statement of a program.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// The value of the expression into a local, which later expressions read as the symbol
    /// (the target of an assignment rule, a derivative of a Jacobian or a common subexpression).
    Let(Symbol, Expr),
    /// The next output of the function.
    Output(Expr),
}

/// The programs of a function of an event index: the assignment rules before the branches,
/// and one program of outputs per event, `None` for an event without the part (a priority, a
/// delay).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Branches {
    /// The assignment rules any branch reads (only lets).
    pub prefix: Program,
    /// The outputs of every event.
    pub branches: Vec<Option<Program>>,
}

/// The programs of the functions of a model; the outputs are those of the code as written, in
/// its order.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// `model_init`: one program per initial evaluation, its output the value (the memory
    /// changes between them, so each is a function of its own).
    pub init: Vec<Program>,
    /// `model_rhs`: the rates.
    pub rhs: Program,
    /// `model_jac`: the entries of an analytic Jacobian.
    pub jac: Option<Program>,
    /// `model_roots`: the root functions of the triggers, then those of the discontinuities
    /// ([`Model::discontinuity_roots`]).
    pub roots: Program,
    /// `model_trigger`: the trigger of every event.
    pub trigger: Branches,
    /// `model_priority`.
    pub priority: Branches,
    /// `model_delay`.
    pub delay: Branches,
    /// `model_event_values`: the values of the assignments of every event.
    pub event_values: Branches,
    /// `model_selections`: every selection.
    pub selections: Program,
    /// `model_assign`: the value of every assignment rule in its order, which the code stores
    /// into the memory before the next one (so a later rule reads the memory).
    pub assign: Program,
}

/// The programs of the functions of a model and of its Jacobian, with the common
/// subexpressions when `cse`.
///
/// The root functions are those of the triggers of `model` as written, folded each
/// ([`Event::root_functions`] of a folded trigger may be fewer), then those of the
/// discontinuities ([`Model::discontinuity_roots`], folded already).
pub fn plan(model: &Model, jacobian: &JacobianIr, cse: bool) -> Plan {
    let rules = &model.assignment_rules;
    let mut init_commons = 0;
    let init = model
        .initial
        .iter()
        .map(|a| program(&[], std::slice::from_ref(&a.expr), cse, &mut init_commons))
        .collect();
    let jac = match jacobian {
        JacobianIr::Analytic {
            derivatives,
            entries,
            ..
        } => {
            let lets: Vec<Assignment> = rules
                .iter()
                .cloned()
                .chain(derivatives.iter().enumerate().map(|(k, d)| Assignment {
                    target: Symbol::Derivative(index(k)),
                    expr: d.clone(),
                }))
                .collect();
            Some(program(&lets, entries, cse, &mut 0))
        }
        JacobianIr::ColoredDifference { .. } => None,
    };
    let roots: Vec<Expr> = model
        .events
        .iter()
        .flat_map(Event::root_functions)
        .chain(model.discontinuity_roots())
        .collect();
    let part = |f: &dyn Fn(&Event) -> Option<Vec<Expr>>| -> Branches {
        let outputs: Vec<Option<Vec<Expr>>> = model.events.iter().map(f).collect();
        branches(rules, &outputs, cse)
    };
    let selections: Vec<Expr> = model.selections.iter().map(|s| s.expr.clone()).collect();
    let values: Vec<Expr> = rules.iter().map(|a| a.expr.clone()).collect();
    Plan {
        init,
        rhs: program(rules, &model.rates, cse, &mut 0),
        jac,
        roots: program(rules, &roots, cse, &mut 0),
        trigger: part(&|e| Some(vec![e.trigger.clone()])),
        priority: part(&|e| e.priority.clone().map(|x| vec![x])),
        delay: part(&|e| e.delay.clone().map(|x| vec![x])),
        event_values: part(&|e| Some(e.assignments.iter().map(|a| a.value.clone()).collect())),
        selections: program(rules, &selections, cse, &mut 0),
        assign: program(&[], &values, cse, &mut 0),
    }
}

/// The program of a function which evaluates `lets` into locals in their order (each reads only
/// those before it) and then `outputs`: every expression folded ([`fold`]), only the lets the
/// outputs read (directly or through later lets), and with the common subexpressions when
/// `cse`, numbered from `commons`, which is advanced past them.
pub fn program(lets: &[Assignment], outputs: &[Expr], cse: bool, commons: &mut u32) -> Program {
    let outputs: Vec<Expr> = outputs.iter().map(fold).collect();
    let mut roots = live(lets, &outputs);
    roots.extend(outputs.into_iter().map(Root::Output));
    common_subexpressions(roots, cse, commons)
}

/// The programs of a function of an event index ([`Branches`]): the lets any branch reads, then
/// per event its outputs, each a scope of common subexpressions of its own.
pub fn branches(lets: &[Assignment], outputs: &[Option<Vec<Expr>>], cse: bool) -> Branches {
    let outputs: Vec<Option<Vec<Expr>>> = outputs
        .iter()
        .map(|o| o.as_ref().map(|o| o.iter().map(fold).collect()))
        .collect();
    let all: Vec<Expr> = outputs.iter().flatten().flatten().cloned().collect();
    let mut commons = 0;
    let prefix = common_subexpressions(live(lets, &all), cse, &mut commons);
    let branches = outputs
        .into_iter()
        .map(|o| {
            o.map(|o| {
                common_subexpressions(o.into_iter().map(Root::Output).collect(), cse, &mut commons)
            })
        })
        .collect();
    Branches { prefix, branches }
}

/// A statement before the common subexpressions.
enum Root {
    Let(Symbol, Expr),
    Output(Expr),
}

impl Root {
    fn expr(&self) -> &Expr {
        match self {
            Root::Let(_, e) | Root::Output(e) => e,
        }
    }
}

/// The folded lets the outputs read, directly or through later lets, in their order.
fn live(lets: &[Assignment], outputs: &[Expr]) -> Vec<Root> {
    let mut read: HashSet<Symbol> = HashSet::new();
    for o in outputs {
        o.visit_symbols(&mut |s| {
            read.insert(s);
        });
    }
    let mut kept = Vec::new();
    for a in lets.iter().rev() {
        if read.contains(&a.target) {
            let e = fold(&a.expr);
            e.visit_symbols(&mut |s| {
                read.insert(s);
            });
            kept.push(Root::Let(a.target, e));
        }
    }
    kept.reverse();
    kept
}

/// Whether an expression is a literal number or boolean.
fn is_literal(e: &Expr) -> bool {
    matches!(e, Expr::Number(_) | Expr::Bool(_))
}

/// Whether an expression is the number of the bits of `x`.
fn is_number(e: &Expr, x: f64) -> bool {
    matches!(e, Expr::Number(v) if v.to_bits() == x.to_bits())
}

/// The value of an operator of literal operands as the emitted code computes it.
fn literal(op: Op, args: Vec<Expr>) -> Expr {
    let e = Expr::Apply { op, args };
    let read = |s: Symbol| unreachable!("{s:?} in literal operands");
    if is_boolean(op) {
        Expr::Bool(eval_bool(&e, &read))
    } else {
        Expr::Number(eval(&e, &read))
    }
}

/// The expression with its constants folded and the rewrites which are exact in binary64 (see
/// the module documentation), bottom up.
pub fn fold(e: &Expr) -> Expr {
    match e {
        Expr::Number(_) | Expr::Bool(_) | Expr::Symbol(_) => e.clone(),
        Expr::Apply { op, args } => fold_apply(*op, args.iter().map(fold).collect()),
        Expr::Piecewise { pieces, otherwise } => {
            let mut kept: Vec<(Expr, Expr)> = Vec::new();
            for (value, condition) in pieces {
                let condition = fold(condition);
                if !is_literal(&condition) {
                    kept.push((fold(value), condition));
                    continue;
                }
                if eval_bool(&condition, &|s| unreachable!("{s:?} in a literal")) {
                    let value = fold(value);
                    return if kept.is_empty() {
                        value
                    } else {
                        Expr::Piecewise {
                            pieces: kept,
                            otherwise: Some(Box::new(value)),
                        }
                    };
                }
                // a literal false: never taken
            }
            let otherwise = otherwise.as_deref().map(fold);
            if kept.is_empty() {
                otherwise.unwrap_or(Expr::Number(f64::NAN))
            } else {
                Expr::Piecewise {
                    pieces: kept,
                    otherwise: otherwise.map(Box::new),
                }
            }
        }
    }
}

/// An operator of folded arguments, folded.
fn fold_apply(op: Op, mut args: Vec<Expr>) -> Expr {
    if args.iter().all(is_literal) {
        return literal(op, args);
    }
    match op {
        Op::Plus | Op::Times | Op::Max | Op::Min => {
            // the leading literal numbers are the first operations of the left fold
            let lead = args
                .iter()
                .take_while(|a| matches!(a, Expr::Number(_)))
                .count();
            if lead >= 2 {
                let first = literal(op, args.drain(..lead).collect());
                args.insert(0, first);
            }
            if op == Op::Times {
                // a factor 1 (a symbol or an operator remains, which is no literal)
                args.retain(|a| !is_number(a, 1.0));
            }
            if args.len() == 1 {
                return args.pop().expect("one argument");
            }
            if matches!(op, Op::Max | Op::Min) {
                let mut rest = args.into_iter();
                let first = rest.next().expect("more than one argument");
                return rest.fold(first, |acc, b| Expr::apply(op, vec![acc, b]));
            }
            Expr::apply(op, args)
        }
        Op::Minus if args.len() == 2 && is_number(&args[1], 0.0) => args.swap_remove(0),
        Op::Divide if args.len() == 2 && is_number(&args[1], 1.0) => args.swap_remove(0),
        _ => Expr::apply(op, args),
    }
}

/// A node of the expressions of a function: equal subtrees (with the bits of their numbers) are
/// one node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Node {
    Number(u64),
    Bool(bool),
    Symbol(Symbol),
    Apply(Op, Vec<u32>),
    /// The children `value, condition` of every piece, then the `otherwise` when the flag is set.
    Piecewise(Vec<u32>, bool),
}

impl Node {
    fn children(&self) -> &[u32] {
        match self {
            Node::Apply(_, c) | Node::Piecewise(c, _) => c,
            _ => &[],
        }
    }

    /// Whether the child at this position is evaluated whenever the node is: every argument of
    /// an operator, of a `piecewise` only the first condition.
    fn eager(&self, position: usize) -> bool {
        match self {
            Node::Piecewise(..) => position == 1,
            _ => true,
        }
    }

    /// Whether the node may be computed into a local: an operator of a double or a `piecewise`.
    fn shareable(&self) -> bool {
        match self {
            Node::Apply(op, _) => !is_boolean(*op),
            Node::Piecewise(..) => true,
            _ => false,
        }
    }
}

/// The nodes of the expressions of a function; a child has a smaller index than its parent.
#[derive(Default)]
struct Dag {
    nodes: Vec<Node>,
    index: HashMap<Node, u32>,
}

impl Dag {
    fn intern(&mut self, e: &Expr) -> u32 {
        let node = match e {
            Expr::Number(x) => Node::Number(x.to_bits()),
            Expr::Bool(b) => Node::Bool(*b),
            Expr::Symbol(s) => Node::Symbol(*s),
            Expr::Apply { op, args } => {
                Node::Apply(*op, args.iter().map(|a| self.intern(a)).collect())
            }
            Expr::Piecewise { pieces, otherwise } => {
                let mut children = Vec::with_capacity(2 * pieces.len() + 1);
                for (value, condition) in pieces {
                    children.push(self.intern(value));
                    children.push(self.intern(condition));
                }
                if let Some(o) = otherwise {
                    children.push(self.intern(o));
                }
                Node::Piecewise(children, otherwise.is_some())
            }
        };
        if let Some(&id) = self.index.get(&node) {
            return id;
        }
        let id = index(self.nodes.len());
        self.nodes.push(node.clone());
        self.index.insert(node, id);
        id
    }
}

/// The program of the roots with the subtrees which the code would evaluate more than once
/// (one of their evaluations eager) computed once into locals, when `enabled`.
fn common_subexpressions(roots: Vec<Root>, enabled: bool, commons: &mut u32) -> Program {
    let as_statement = |r: Root| match r {
        Root::Let(s, e) => Statement::Let(s, e),
        Root::Output(e) => Statement::Output(e),
    };
    if !enabled {
        return Program {
            statements: roots.into_iter().map(as_statement).collect(),
        };
    }
    let mut dag = Dag::default();
    let ids: Vec<u32> = roots.iter().map(|r| dag.intern(r.expr())).collect();
    let n = dag.nodes.len();
    // how often the code evaluates a node, whether one evaluation is eager, and the shared ones;
    // parents before their children
    let mut evaluations = vec![0u64; n];
    let mut eager = vec![false; n];
    let mut shared = vec![false; n];
    for &id in &ids {
        evaluations[id as usize] += 1;
        eager[id as usize] = true;
    }
    for id in (0..n).rev() {
        if evaluations[id] == 0 {
            continue;
        }
        let node = &dag.nodes[id];
        shared[id] = evaluations[id] >= 2 && eager[id] && node.shareable();
        let times = if shared[id] { 1 } else { evaluations[id] };
        for (position, &c) in node.children().iter().enumerate() {
            let c = c as usize;
            evaluations[c] = evaluations[c].saturating_add(times);
            if eager[id] && node.eager(position) {
                eager[c] = true;
            }
        }
    }

    let mut emitter = Emitter {
        dag: &dag,
        shared: &shared,
        local: vec![None; n],
        queued: vec![false; n],
        scanned: vec![false; n],
        order: Vec::new(),
    };
    let mut statements = Vec::new();
    for (root, &id) in roots.iter().zip(&ids) {
        let i = id as usize;
        match root {
            // a rule whose value is shared is the local of the shared subtree
            Root::Let(symbol, _) if shared[i] && !emitter.queued[i] => {
                emitter.queued[i] = true;
                emitter.scan(id);
                emitter.flush(&mut statements, commons);
                statements.push(Statement::Let(*symbol, emitter.build(id, true)));
                emitter.local[i] = Some(*symbol);
            }
            _ => {
                emitter.visit(id);
                emitter.flush(&mut statements, commons);
                let e = emitter.build(id, false);
                statements.push(match root {
                    Root::Let(symbol, _) => Statement::Let(*symbol, e),
                    Root::Output(_) => Statement::Output(e),
                });
            }
        }
    }
    Program { statements }
}

/// The definitions of the shared nodes in the order of their first use.
struct Emitter<'d> {
    dag: &'d Dag,
    shared: &'d [bool],
    /// The symbol of the local of a defined shared node.
    local: Vec<Option<Symbol>>,
    /// Whether a shared node is defined or about to be.
    queued: Vec<bool>,
    /// Whether the shared nodes below a node which is not shared are queued.
    scanned: Vec<bool>,
    /// The shared nodes to define before the next statement, children first.
    order: Vec<u32>,
}

impl Emitter<'_> {
    /// Queues the shared nodes an evaluation of the node needs, the node itself when shared.
    fn visit(&mut self, id: u32) {
        let i = id as usize;
        if self.shared[i] {
            if !self.queued[i] {
                self.queued[i] = true;
                self.scan(id);
                self.order.push(id);
            }
        } else if !self.scanned[i] {
            self.scanned[i] = true;
            self.scan(id);
        }
    }

    /// Queues the shared nodes the children of the node need.
    fn scan(&mut self, id: u32) {
        for &c in self.dag.nodes[id as usize].children() {
            self.visit(c);
        }
    }

    /// The definitions of the queued nodes.
    fn flush(&mut self, statements: &mut Vec<Statement>, commons: &mut u32) {
        for id in std::mem::take(&mut self.order) {
            let symbol = Symbol::Common(*commons);
            *commons += 1;
            statements.push(Statement::Let(symbol, self.build(id, true)));
            self.local[id as usize] = Some(symbol);
        }
    }

    /// The expression of a node, which reads the locals of the shared nodes below it (and of
    /// itself unless `definition`).
    fn build(&self, id: u32, definition: bool) -> Expr {
        let i = id as usize;
        if !definition && let Some(symbol) = self.local[i] {
            return Expr::Symbol(symbol);
        }
        let child = |c: u32| self.build(c, false);
        match &self.dag.nodes[i] {
            Node::Number(bits) => Expr::Number(f64::from_bits(*bits)),
            Node::Bool(b) => Expr::Bool(*b),
            Node::Symbol(s) => Expr::Symbol(*s),
            Node::Apply(op, args) => Expr::apply(*op, args.iter().map(|&c| child(c)).collect()),
            Node::Piecewise(children, has_otherwise) => {
                let (pairs, otherwise) = if *has_otherwise {
                    let (o, rest) = children.split_last().expect("an otherwise");
                    (rest, Some(*o))
                } else {
                    (children.as_slice(), None)
                };
                Expr::Piecewise {
                    pieces: pairs
                        .chunks(2)
                        .map(|p| (child(p[0]), child(p[1])))
                        .collect(),
                    otherwise: otherwise.map(|o| Box::new(child(o))),
                }
            }
        }
    }
}
