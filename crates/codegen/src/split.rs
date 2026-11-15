//! The functions of `model.wasm` as lists of statements, split into helper functions when they
//! are above the limits ([`MAX_FUNCTION_BYTES`](crate::MAX_FUNCTION_BYTES),
//! [`MAX_LOCALS`](crate::MAX_LOCALS)).
//!
//! A function of the code generation is a list of statements ([`Stmt`]): a let (the value of an
//! expression into a local, which later statements read for its symbol: an assignment rule, a
//! derivative of a Jacobian, a common subexpression), or an output (a store into the memory, a
//! block of `model_init`). The expressions are pure and read only `t`, `y`, `p` and the lets,
//! so computing a let again gives its bits.
//!
//! **Locals.** The local of a let is given back after the last statement which reads it, and a
//! later let or temporary reuses it, so the number of locals is the most values alive at once
//! and not the number of lets (the common subexpressions of the optimizer are many).
//!
//! **A function within the limits** is emitted as one function, every let where it stands.
//!
//! **A function above the limits** ([`ModuleBuilder::sequence`]) is cut into helper functions of
//! the same parameters, which the function calls in their order (a chain: `f(t, y, p, out)` is
//! `f_1(t, y, p, out); f_2(t, y, p, out); ...`). Its statements are emitted in their order into
//! the current helper, and a statement which puts the helper above the limits is taken back and
//! starts the next helper. A let is computed in a helper right before the first of its
//! statements which reads it (with the lets it reads, in their order), and in every helper
//! which reads it again: no value passes from one helper to the next, neither through the memory
//! (the model writes nothing into the memory of the engine but where it is told) nor through
//! globals (a model holds no state). The outputs are those of the function as written in the
//! same order, and a let computed later or again gives the same bits, so a split function gives
//! the bits of the function as one. The functions whose outputs write the memory which their
//! expressions read keep that order: `model_init` splits only between its blocks (each block
//! with its lets inside), and `model_assign` stores rule after rule; its common subexpressions
//! read only the targets of the rules stored before the first rule which reads them (the rules
//! are in their order of evaluation), so they read the same memory where they are computed again.
//!
//! **A function of an event index** ([`ModuleBuilder::indexed`]: `model_trigger`, `..._priority`,
//! `..._delay`, `..._event_values`, `..._event_apply`) above the limits is cut into helpers of
//! consecutive events: the function dispatches on the index (`if i < end_1 { return f_1(...) }
//! ...`), and the branch of an event in a helper computes the assignment rules it reads inside
//! the branch, where only the one branch of the index runs.
//!
//! A statement which alone (with the lets it reads) is above the limits is an error: a `model_jac`
//! as written with such an entry is the colored difference quotient under `Auto`.

use std::collections::{BTreeMap, HashMap, HashSet};

use sbmlsim_ir::{CompileError, Expr, Symbol};
use wasm_encoder::ValType;

use crate::wasm::{Body, Ctx, ModuleBuilder};

/// A statement of a function.
pub(crate) enum Stmt<'e> {
    /// The value of the expression into a local, which later statements read as the symbol.
    Let(Symbol, &'e Expr),
    /// `out[k] = expr`, `out` the pointer in the local `ptr`.
    StoreAt { ptr: u32, k: u32, expr: &'e Expr },
    /// `p[i] = expr` or `y[i] = expr`.
    Store(Symbol, &'e Expr),
    /// `p[i] = x` or `y[i] = x`.
    Constant(Symbol, f64),
    /// `out[k] = x` for the `i32` vector in the local `ptr`.
    I32At { ptr: u32, k: u32, x: u32 },
    /// `if fixed[k] == 0 { body }`: a scope of its own, whose lets are not read outside.
    Unfixed {
        fixed: u32,
        k: u32,
        body: Vec<Stmt<'e>>,
    },
}

impl<'e> Stmt<'e> {
    /// The symbol a let defines.
    fn defines(&self) -> Option<Symbol> {
        match self {
            Stmt::Let(s, _) => Some(*s),
            _ => None,
        }
    }

    /// Calls `f` on every symbol the statement reads (in a block: every symbol of its body).
    fn visit_symbols(&self, f: &mut dyn FnMut(Symbol)) {
        match self {
            Stmt::Let(_, e) | Stmt::StoreAt { expr: e, .. } | Stmt::Store(_, e) => {
                e.visit_symbols(&mut |s| f(s))
            }
            Stmt::Constant(..) | Stmt::I32At { .. } => {}
            Stmt::Unfixed { body, .. } => body.iter().for_each(|s| s.visit_symbols(f)),
        }
    }

    /// Emits the statement (the lets of a block where they stand).
    fn emit(&self, f: &mut Body<'_>) {
        match self {
            Stmt::Let(s, e) => f.let_(*s, e),
            Stmt::StoreAt { ptr, k, expr } => f.store_at(*ptr, *k, expr),
            Stmt::Store(target, e) => f.store(*target, e),
            Stmt::Constant(target, x) => f.store_constant(*target, *x),
            Stmt::I32At { ptr, k, x } => f.store_i32_at(*ptr, *k, *x),
            Stmt::Unfixed { fixed, k, body } => {
                f.if_not_fixed(*fixed, *k);
                emit_in_place(f, body);
                f.end();
            }
        }
    }
}

/// How a branch of an event index ends.
pub(crate) enum End<'e> {
    /// `return expr` as a boolean (`i32`).
    Bool(&'e Expr),
    /// `return expr` as an `f64`.
    F64(&'e Expr),
    /// `return`.
    Return,
}

/// The branch of an event in a function of an event index.
pub(crate) enum Branch<'e> {
    /// Statements (their own lets and outputs), then the end.
    Program { stmts: Vec<Stmt<'e>>, end: End<'e> },
    /// Code of its own (`model_event_apply`), which reads none of the lets before the branches,
    /// then `return`.
    Custom(Box<dyn Fn(&mut Body<'_>) + 'e>),
}

/// What a function of an event index does after the branches (an index of no event, or of an
/// event without the part).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Tail {
    Nothing,
    I32Zero,
    F64Zero,
}

impl Tail {
    /// The bytes of the code of the tail ([`emit_tail`]).
    fn bytes(self) -> usize {
        match self {
            Tail::Nothing => 0,
            // i32.const 0
            Tail::I32Zero => 2,
            // f64.const 0, return
            Tail::F64Zero => 10,
        }
    }
}

/// The signature of a function of `model/1`.
pub(crate) struct Signature<'a> {
    pub export: &'static str,
    pub params: &'a [ValType],
    pub results: &'a [ValType],
    pub ctx: Ctx,
}

/// A list of statements and what the lets of the top level of it are read by.
struct Liveness {
    /// The position of the let of a symbol.
    def: HashMap<Symbol, usize>,
    /// The lets each statement reads (defined before it), each once.
    reads: Vec<Vec<Symbol>>,
    /// The lets to give back after each statement (after the last statement which reads them,
    /// or after the let itself when nothing reads it).
    release_after: Vec<Vec<Symbol>>,
    /// The last statement which reads the let at each position (itself when none).
    last: Vec<usize>,
}

impl Liveness {
    /// The liveness of a list of statements.
    fn of(stmts: &[Stmt<'_>]) -> Liveness {
        Self::of_items(
            stmts.len(),
            |i, f| stmts[i].visit_symbols(f),
            |i| stmts[i].defines(),
        )
    }

    /// The liveness of `n` items, `visit(i, f)` calling `f` on the symbols item `i` reads and
    /// `defines(i)` the symbol of a let.
    fn of_items(
        n: usize,
        visit: impl Fn(usize, &mut dyn FnMut(Symbol)),
        defines: impl Fn(usize) -> Option<Symbol>,
    ) -> Liveness {
        let mut def = HashMap::new();
        let mut reads = Vec::with_capacity(n);
        let mut last: Vec<usize> = (0..n).collect();
        let mut seen = HashSet::new();
        for i in 0..n {
            let mut read = Vec::new();
            seen.clear();
            visit(i, &mut |s| {
                if let Some(&d) = def.get(&s)
                    && seen.insert(s)
                {
                    read.push(s);
                    last[d] = i;
                }
            });
            reads.push(read);
            if let Some(s) = defines(i) {
                let previous = def.insert(s, i);
                debug_assert!(previous.is_none(), "{s:?} is defined twice");
            }
        }
        let mut release_after = vec![Vec::new(); n];
        for (&s, &d) in &def {
            release_after[last[d]].push(s);
        }
        // the order of the releases is the order of the lets (the free list, and so the locals
        // of the code, do not depend on the order of a hash map)
        for list in &mut release_after {
            list.sort_by_key(|s| def[s]);
        }
        Liveness {
            def,
            reads,
            release_after,
            last,
        }
    }
}

/// Emits statements where they stand into `f`, each let where it stands and given back after the
/// last statement which reads it.
fn emit_in_place(f: &mut Body<'_>, stmts: &[Stmt<'_>]) {
    let live = Liveness::of(stmts);
    for (i, s) in stmts.iter().enumerate() {
        s.emit(f);
        for symbol in &live.release_after[i] {
            f.release_let(*symbol);
        }
    }
}

/// Emits a branch of an event index into `f`: `if i == k { ...; return }`.
fn emit_branch(f: &mut Body<'_>, k: u32, branch: &Branch<'_>, before: &[Stmt<'_>]) {
    f.if_index(0, k);
    match branch {
        Branch::Program { stmts, end } => {
            let all: Vec<&Stmt<'_>> = before.iter().chain(stmts.iter()).collect();
            let end_expr = match end {
                End::Bool(e) | End::F64(e) => Some(*e),
                End::Return => None,
            };
            emit_refs(f, &all, end_expr);
            match end {
                End::Bool(e) => f.return_bool(e),
                End::F64(e) => f.return_f64(e),
                End::Return => f.return_(),
            }
            // the lets of the branch are not read after it
            for s in all.iter().filter_map(|s| s.defines()) {
                f.release_let(s);
            }
        }
        Branch::Custom(code) => {
            code(f);
            f.return_();
        }
    }
    f.end();
}

/// [`emit_in_place`] of statements by reference with the expression of an end.
fn emit_refs(f: &mut Body<'_>, stmts: &[&Stmt<'_>], end: Option<&Expr>) {
    let n = stmts.len() + usize::from(end.is_some());
    let live = Liveness::of_items(
        n,
        |i, g| match stmts.get(i) {
            Some(s) => s.visit_symbols(g),
            None => end.expect("the end").visit_symbols(&mut |s| g(s)),
        },
        |i| stmts.get(i).and_then(|s| s.defines()),
    );
    for (i, s) in stmts.iter().enumerate() {
        s.emit(f);
        for symbol in &live.release_after[i] {
            f.release_let(*symbol);
        }
    }
}

impl ModuleBuilder {
    /// A function of statements run in their order (without a result), split into a chain of
    /// helper functions when it is above the limits.
    pub(crate) fn sequence(
        &mut self,
        sig: &Signature<'_>,
        stmts: &[Stmt<'_>],
    ) -> Result<(), CompileError> {
        let mut f = self.body(Some(sig.export), sig.params, sig.results, sig.ctx);
        emit_in_place(&mut f, stmts);
        if f.within_limits() {
            f.finish()?;
            return Ok(());
        }
        drop(f);
        self.split_sequence(sig, stmts)
    }

    /// [`ModuleBuilder::sequence`] above the limits: a chain of helper functions, each let
    /// computed right before the first statement of a helper which reads it.
    fn split_sequence(
        &mut self,
        sig: &Signature<'_>,
        stmts: &[Stmt<'_>],
    ) -> Result<(), CompileError> {
        let live = Liveness::of(stmts);
        let mut helpers: Vec<u32> = Vec::new();
        let mut chunk = Chunk::new(self, sig);
        for (j, s) in stmts.iter().enumerate() {
            if matches!(s, Stmt::Let(..)) {
                continue;
            }
            let mark = chunk.f.mark();
            chunk.statement(stmts, &live, j);
            if chunk.f.within_limits() {
                chunk.statements += 1;
                continue;
            }
            if chunk.statements == 0 {
                return Err(chunk.f.above_the_limits(sig.export, "one statement: "));
            }
            chunk.f.rollback(mark);
            helpers.push(chunk.finish()?);
            chunk = Chunk::new(self, sig);
            chunk.statement(stmts, &live, j);
            if !chunk.f.within_limits() {
                return Err(chunk.f.above_the_limits(sig.export, "one statement: "));
            }
            chunk.statements += 1;
        }
        helpers.push(chunk.finish()?);
        let mut f = self.body(Some(sig.export), sig.params, sig.results, sig.ctx);
        for helper in helpers {
            f.call_helper(helper);
        }
        f.finish()?;
        Ok(())
    }

    /// A function of an event index (the local 0): the lets of `prefix`, then per event its
    /// branch, `if i == k { ... return }`, then `tail`; split into helper functions of
    /// consecutive events, which the function dispatches to by the index, when it is above the
    /// limits.
    pub(crate) fn indexed(
        &mut self,
        sig: &Signature<'_>,
        prefix: &[Stmt<'_>],
        branches: &[Option<Branch<'_>>],
        tail: Tail,
    ) -> Result<(), CompileError> {
        let mut f = self.body(Some(sig.export), sig.params, sig.results, sig.ctx);
        emit_indexed_in_place(&mut f, prefix, branches);
        emit_tail(&mut f, tail);
        if f.within_limits() {
            f.finish()?;
            return Ok(());
        }
        drop(f);
        // the lets of the prefix each branch reads, with the lets they read, in their order
        let live = Liveness::of(prefix);
        let needed = |branch: &Branch<'_>| -> Vec<usize> {
            let Branch::Program { stmts, end } = branch else {
                return Vec::new();
            };
            let mut wanted = HashSet::new();
            let mut stack: Vec<usize> = Vec::new();
            let mut visit = |s: Symbol| {
                if let Some(&d) = live.def.get(&s)
                    && wanted.insert(d)
                {
                    stack.push(d);
                }
            };
            for s in stmts {
                s.visit_symbols(&mut visit);
            }
            if let End::Bool(e) | End::F64(e) = end {
                e.visit_symbols(&mut visit);
            }
            while let Some(d) = stack.pop() {
                for &s in &live.reads[d] {
                    let d = live.def[&s];
                    if wanted.insert(d) {
                        stack.push(d);
                    }
                }
            }
            let mut positions: Vec<usize> = wanted.into_iter().collect();
            positions.sort_unstable();
            positions
        };
        let mut helpers: Vec<(u32, u32)> = Vec::new();
        let mut f = self.body(None, sig.params, sig.results, sig.ctx);
        let mut in_helper = 0;
        for (k, branch) in branches.iter().enumerate() {
            let Some(branch) = branch else { continue };
            let before: Vec<Stmt<'_>> = needed(branch)
                .into_iter()
                .map(|d| match &prefix[d] {
                    Stmt::Let(s, e) => Stmt::Let(*s, e),
                    _ => unreachable!("the prefix has only lets"),
                })
                .collect();
            let mark = f.mark();
            emit_branch(&mut f, k as u32, branch, &before);
            // the helper ends with the tail
            if f.within_limits_with(tail.bytes()) {
                in_helper += 1;
                continue;
            }
            if in_helper == 0 {
                return Err(f.above_the_limits(sig.export, "one event: "));
            }
            f.rollback(mark);
            emit_tail(&mut f, tail);
            let helper = f.finish()?;
            helpers.push((helper, k as u32));
            f = self.body(None, sig.params, sig.results, sig.ctx);
            emit_branch(&mut f, k as u32, branch, &before);
            if !f.within_limits_with(tail.bytes()) {
                return Err(f.above_the_limits(sig.export, "one event: "));
            }
            in_helper = 1;
        }
        emit_tail(&mut f, tail);
        let helper = f.finish()?;
        helpers.push((helper, branches.len() as u32));
        // the dispatch: the helper of the first range which holds the index
        let mut f = self.body(Some(sig.export), sig.params, sig.results, sig.ctx);
        for (helper, end) in helpers {
            f.if_index_below(end);
            f.call_helper(helper);
            f.return_();
            f.end();
        }
        emit_tail(&mut f, tail);
        f.finish()?;
        Ok(())
    }
}

/// The function of an event index as one function: the lets of the prefix, then the branches.
fn emit_indexed_in_place(f: &mut Body<'_>, prefix: &[Stmt<'_>], branches: &[Option<Branch<'_>>]) {
    // the lets of the prefix are given back after the last branch which reads them
    let ends: Vec<&Branch<'_>> = branches.iter().flatten().collect();
    let live = Liveness::of_items(
        prefix.len() + ends.len(),
        |i, g| match prefix.get(i) {
            Some(s) => s.visit_symbols(g),
            None => match ends[i - prefix.len()] {
                Branch::Program { stmts, end } => {
                    stmts.iter().for_each(|s| s.visit_symbols(g));
                    if let End::Bool(e) | End::F64(e) = end {
                        e.visit_symbols(&mut |s| g(s));
                    }
                }
                Branch::Custom(_) => {}
            },
        },
        |i| prefix.get(i).and_then(Stmt::defines),
    );
    for (i, s) in prefix.iter().enumerate() {
        s.emit(f);
        for symbol in &live.release_after[i] {
            f.release_let(*symbol);
        }
    }
    let mut position = prefix.len();
    for (k, branch) in branches.iter().enumerate() {
        let Some(branch) = branch else { continue };
        emit_branch(f, k as u32, branch, &[]);
        for symbol in &live.release_after[position] {
            f.release_let(*symbol);
        }
        position += 1;
    }
}

/// The code after the branches.
fn emit_tail(f: &mut Body<'_>, tail: Tail) {
    match tail {
        Tail::Nothing => {}
        Tail::I32Zero => f.i32_const(0),
        Tail::F64Zero => f.return_f64(&Expr::Number(0.0)),
    }
}

/// A helper function of a split sequence being built.
struct Chunk<'m> {
    f: Body<'m>,
    /// The statements (but lets) in the helper.
    statements: usize,
    /// The lets to give back after a statement, by its position (given back after the first
    /// statement of the helper at or after it).
    release_at: BTreeMap<usize, Vec<Symbol>>,
}

impl<'m> Chunk<'m> {
    fn new(m: &'m mut ModuleBuilder, sig: &Signature<'_>) -> Chunk<'m> {
        Chunk {
            f: m.body(None, sig.params, sig.results, sig.ctx),
            statements: 0,
            release_at: BTreeMap::new(),
        }
    }

    /// Emits statement `j` (no let) with the lets it reads which the helper has not computed
    /// (with the lets they read, depth first, so in an order in which each reads only those
    /// before it), then gives back the lets read for the last time.
    fn statement(&mut self, stmts: &[Stmt<'_>], live: &Liveness, j: usize) {
        let mut stack: Vec<(Symbol, bool)> = live.reads[j]
            .iter()
            .rev()
            .filter(|s| !self.f.has_let(**s))
            .map(|s| (*s, false))
            .collect();
        let mut expanded = HashSet::new();
        while let Some((s, ready)) = stack.pop() {
            if self.f.has_let(s) {
                continue;
            }
            let d = live.def[&s];
            if ready {
                let Stmt::Let(_, e) = &stmts[d] else {
                    unreachable!("a let")
                };
                self.f.let_(s, e);
                // given back after its last statement, at the earliest after this one
                let at = live.last[d].max(j);
                self.release_at.entry(at).or_default().push(s);
            } else if expanded.insert(s) {
                stack.push((s, true));
                for r in live.reads[d].iter().rev() {
                    if !self.f.has_let(*r) {
                        stack.push((*r, false));
                    }
                }
            }
        }
        stmts[j].emit(&mut self.f);
        let later = self.release_at.split_off(&(j + 1));
        for s in std::mem::replace(&mut self.release_at, later)
            .into_values()
            .flatten()
        {
            self.f.release_let(s);
        }
    }

    fn finish(self) -> Result<u32, CompileError> {
        self.f.finish()
    }
}
