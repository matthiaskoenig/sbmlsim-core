//! DDE-only private functions. Each history site locates its record with binary search and
//! interpolates only the states its operand and its assignment rules actually read.
use super::*;
use crate::{VECTORS, fixed_index};
use sbmlsim_abi::history as h;
use sbmlsim_ir::{Model, optimize::tree_key};
use std::collections::HashSet;
use wasm_encoder::ValType::{F64, I32};

mod projection;

const LOCATE: u32 = 0;
const INTERPOLATE: u32 = 1;
const FIRST_OPERAND: u32 = 2;

fn walk(e: &Expr, visit: &mut impl FnMut(&Expr)) {
    visit(e);
    match e {
        Expr::Apply { args, .. } => args.iter().for_each(|a| walk(a, visit)),
        Expr::Piecewise { pieces, otherwise } => {
            for (v, c) in pieces {
                walk(v, visit);
                walk(c, visit);
            }
            if let Some(o) = otherwise {
                walk(o, visit);
            }
        }
        _ => {}
    }
}

pub(crate) fn delay_sites(model: &Model, roots: &[Expr]) -> Vec<Expr> {
    let mut keys = HashSet::new();
    let mut sites = Vec::new();
    let mut collect = |e: &Expr| {
        walk(e, &mut |e| {
            if let Expr::Apply {
                op: Op::Delay,
                args,
            } = e
                && keys.insert(tree_key(&args[0]))
            {
                sites.push(args[0].clone());
            }
        })
    };
    for a in model.initial.iter().chain(&model.assignment_rules) {
        collect(&a.expr);
    }
    for e in &model.rates {
        collect(e);
    }
    for s in &model.selections {
        collect(&s.expr);
    }
    for e in &model.events {
        collect(&e.trigger);
        for a in &e.assignments {
            collect(&a.value);
        }
        for x in e.delay.iter().chain(&e.priority) {
            collect(x);
        }
    }
    for e in roots {
        collect(e);
    }
    sites
}

// Dependencies at this time. Every nested delay recomputes its own historical operands.
fn reads(e: &Expr, needed: &mut HashSet<Symbol>) {
    match e {
        Expr::Symbol(s) => {
            needed.insert(*s);
        }
        Expr::Apply {
            op: Op::Delay,
            args,
        } => {
            reads(&args[1], needed);
            if !matches!(&args[1], Expr::Number(n) if *n > 0.0) {
                reads(&args[0], needed);
            }
        }
        Expr::Apply { args, .. } => args.iter().for_each(|a| reads(a, needed)),
        Expr::Piecewise { pieces, otherwise } => {
            for (v, c) in pieces {
                reads(v, needed);
                reads(c, needed);
            }
            if let Some(o) = otherwise {
                reads(o, needed);
            }
        }
        _ => {}
    }
}

pub(crate) fn closure<'a>(assignments: &'a [Assignment], operand: &Expr) -> Vec<&'a Assignment> {
    live_assignments(assignments, std::iter::once(operand))
}

pub(crate) fn live_assignments<'a, 'e>(
    assignments: &'a [Assignment],
    operands: impl Iterator<Item = &'e Expr>,
) -> Vec<&'a Assignment> {
    let mut needed = HashSet::new();
    for operand in operands {
        reads(operand, &mut needed);
    }
    let mut result = Vec::new();
    for a in assignments.iter().rev() {
        if needed.remove(&a.target) {
            reads(&a.expr, &mut needed);
            result.push(a);
        }
    }
    result.reverse();
    result
}

impl ModuleBuilder {
    pub(crate) fn history(&mut self, model: &Model, sites: &[Expr]) -> Result<(), CompileError> {
        if sites.is_empty() {
            return Ok(());
        }
        self.constants = model.parameters.len() as u32;
        self.delay_indices = sites
            .iter()
            .enumerate()
            .map(|(i, e)| (tree_key(e), i as u32))
            .collect();
        let ny = model.states.len() as u32;
        let mut locate = self.body(None, &[F64, I32], &[I32], VECTORS);
        locate.locate();
        locate.finish()?;
        let mut interpolate = self.body(None, &[F64, I32, I32], &[F64], VECTORS);
        interpolate.interpolate_state(ny);
        interpolate.finish()?;
        for operand in sites {
            let mut f = self.body(None, &[F64, I32, I32, F64], &[F64], VECTORS);
            f.operand(model, operand)?;
            f.finish()?;
        }
        Ok(())
    }
}

impl Body<'_> {
    /// The header address for the current evaluation frame, as a Wasm i32.
    fn header_address(&mut self) {
        let p = self.ctx.p;
        let np = self.module.constants;
        self.sink()
            .local_get(p)
            .f64_load(slot(np))
            .i32_trunc_sat_f64_u();
    }

    pub(super) fn query_clock(&mut self, lag: &Expr) {
        let t = self.ctx.t;
        let value = self.temp();
        self.f64(lag);
        self.sink()
            .local_tee(value)
            .f64_const(f64c(0.))
            .f64_ge()
            .local_get(value)
            .f64_const(f64c(f64::INFINITY))
            .f64_lt()
            .i32_and()
            .if_(BlockType::Result(F64))
            .local_get(t)
            .local_get(value)
            .f64_sub();
        self.header_address();
        self.sink()
            .f64_load(byte(h::ACTIVE_SHIFT))
            .f64_add()
            .else_()
            .f64_const(f64c(f64::NAN))
            .end();
        self.release(value);
    }

    /// Signed distances to the surrounding and just-crossed history epochs, held
    /// fixed until the next restart. An inactive query contributes constant roots.
    pub(crate) fn history_roots(&mut self, index: u32, output: u32, expr: &Expr) {
        let query = self.temp();
        let target = self.temp();
        self.f64(expr);
        self.sink().local_set(query);
        self.header_address();
        self.header_address();
        self.sink()
            .i32_load(byte(h::QUERIES))
            .i32_add()
            .local_get(query)
            .f64_store(slot(index));
        for side in 0..h::ROOTS_PER_QUERY as u32 {
            self.sink().local_get(3);
            self.header_address();
            self.header_address();
            self.sink()
                .i32_load(byte(h::TARGETS))
                .i32_add()
                .f64_load(slot(index * h::ROOTS_PER_QUERY as u32 + side))
                .local_tee(target)
                .f64_abs()
                .f64_const(f64c(f64::INFINITY))
                .f64_lt()
                .local_get(query)
                .f64_abs()
                .f64_const(f64c(f64::INFINITY))
                .f64_lt()
                .i32_and()
                .if_(BlockType::Result(F64))
                .local_get(query)
                .local_get(target)
                .f64_sub()
                .else_()
                .f64_const(f64c(if side == 0 { 1. } else { -1. }))
                .end()
                .f64_store(slot(output + side));
        }
        self.release(target);
        self.release(query);
    }

    pub(super) fn history_call(&mut self, args: &[Expr]) {
        // Zero delay sees precisely the caller's current frame, including the assignment
        // locals or the fixed values of the initial assignment currently being evaluated.
        if matches!(&args[1], Expr::Number(lag) if *lag == 0.0) {
            self.f64(&args[0]);
            return;
        }
        let site = self.module.delay_indices[&tree_key(&args[0])];
        let dynamic = !matches!(&args[1], Expr::Number(_));
        let lag = if dynamic {
            self.f64(&args[1]);
            let lag = self.temp();
            self.sink()
                .local_tee(lag)
                .f64_const(f64c(0.))
                .f64_eq()
                .if_(BlockType::Result(F64));
            self.f64(&args[0]);
            self.sink().else_();
            Some(lag)
        } else {
            None
        };
        let c = self.ctx;
        self.sink().local_get(c.t).local_get(c.y).local_get(c.p);
        if let Some(lag) = lag {
            self.sink().local_get(lag);
        } else {
            self.f64(&args[1]);
        }
        self.call_defined(FIRST_OPERAND + site);
        if let Some(lag) = lag {
            self.end();
            self.release(lag);
        }
    }

    // Given a header pointer in local base, push its relative pointer field as an address.
    fn address_field(&mut self, base: u32, field: u32) {
        self.sink()
            .local_get(base)
            .local_get(base)
            .i32_load(byte(field))
            .i32_add();
    }

    fn fail(&mut self, base: u32, reason: u32) {
        // A later speculative query cannot erase an invalid lag or exhausted nesting depth.
        if reason == h::MISSING_RECORD {
            self.sink()
                .local_get(base)
                .i32_load(byte(h::STATUS))
                .i32_eqz()
                .if_(BlockType::Empty);
        }
        self.sink()
            .local_get(base)
            .i32_const(reason as i32)
            .i32_store(byte(h::STATUS));
        if reason == h::MISSING_RECORD {
            self.end();
        }
    }

    fn copy_field(&mut self, destination: u32, base: u32, field: u32, bytes: u32) {
        self.sink().local_get(destination);
        self.address_field(base, field);
        self.sink().i32_const(bytes as i32).memory_copy(0, 0);
    }

    fn operand(&mut self, model: &Model, operand: &Expr) -> Result<(), CompileError> {
        const BASE: u32 = 4;
        const DEPTH: u32 = 5;
        const FIXED: u32 = 6;
        const RECORD: u32 = 7;
        self.integers = 4;
        let caller_shift = self.local();
        let frame_shift = self.local();
        let np = model.parameters.len() as u32;
        let ny = model.states.len() as u32;
        self.sink()
            .local_get(2)
            .f64_load(slot(np))
            .i32_trunc_sat_f64_u()
            .local_tee(BASE)
            .i32_eqz()
            .if_(BlockType::Empty)
            .unreachable()
            .end();
        // The lag is evaluated before entering this function.
        self.sink()
            .local_get(3)
            .f64_const(f64c(0.))
            .f64_ge()
            .local_get(3)
            .f64_const(f64c(f64::INFINITY))
            .f64_lt()
            .i32_and()
            .i32_eqz()
            .if_(BlockType::Empty);
        self.fail(BASE, h::INVALID_LAG);
        self.sink().f64_const(f64c(f64::NAN)).return_().end();
        self.sink()
            .local_get(BASE)
            .local_get(BASE)
            .f64_load(byte(h::MIN_LAG))
            .local_get(3)
            .f64_min()
            .f64_store(byte(h::MIN_LAG));
        self.sink()
            .local_get(BASE)
            .i32_load(byte(h::DEPTH))
            .local_tee(DEPTH)
            .local_get(BASE)
            .i32_load(byte(h::CAPACITY))
            .i32_ge_u()
            .if_(BlockType::Empty);
        self.fail(BASE, h::DEPTH_EXCEEDED);
        self.sink().f64_const(f64c(f64::NAN)).return_().end();
        self.sink()
            .local_get(BASE)
            .local_get(DEPTH)
            .i32_const(1)
            .i32_add()
            .i32_store(byte(h::DEPTH));
        self.address_field(BASE, h::SCRATCH);
        self.sink()
            .local_get(DEPTH)
            .i32_const(((np + ny + 1) * 8) as i32)
            .i32_mul()
            .i32_add()
            .local_tee(1)
            .i32_const((ny * 8) as i32)
            .i32_add()
            .local_set(2);
        self.sink()
            .local_get(0)
            .local_get(3)
            .f64_sub()
            .local_get(BASE)
            .f64_load(byte(h::ACTIVE_SHIFT))
            .local_tee(caller_shift)
            .f64_add()
            .local_set(0)
            .local_get(0)
            .local_get(BASE)
            .f64_load(byte(h::HISTORY_START))
            .f64_lt()
            .if_(BlockType::Result(F64));
        self.sink()
            .local_get(BASE)
            .f64_const(f64c(0.))
            .f64_store(byte(h::ACTIVE_SHIFT));
        self.copy_field(1, BASE, h::DEFAULT_Y, ny * 8);
        self.copy_field(2, BASE, h::DEFAULT_P, np * 8);
        self.sink()
            .local_get(2)
            .local_get(BASE)
            .f64_convert_i32_u()
            .f64_store(slot(np));
        self.address_field(BASE, h::FIXED);
        self.sink().local_set(FIXED);
        self.project_history(model, operand, BASE, FIXED)?;
        for a in closure(&model.initial, operand) {
            self.if_not_fixed(FIXED, fixed_index(model, a.target));
            self.store(a.target, &a.expr);
            self.end();
        }
        self.f64(operand);
        self.sink().else_().local_get(0).local_get(BASE);
        self.call_defined(LOCATE);
        self.sink().local_tee(RECORD).if_(BlockType::Result(F64));
        let rules = closure(&model.assignment_rules, operand);
        let mut used = HashSet::new();
        reads(operand, &mut used);
        for a in rules.iter().rev() {
            used.remove(&a.target);
            reads(&a.expr, &mut used);
        }
        let mut used: Vec<_> = used.into_iter().collect();
        used.sort_unstable();
        let x = self.local();
        if used.iter().any(|s| matches!(s, Symbol::State(_))) {
            self.sink()
                .local_get(0)
                .local_get(RECORD)
                .f64_load(byte(h::END))
                .f64_sub()
                .local_get(RECORD)
                .f64_load(byte(h::H))
                .f64_div()
                .local_set(x);
        }
        self.sink()
            .local_get(2)
            .local_get(BASE)
            .f64_convert_i32_u()
            .f64_store(slot(np));
        // Coordinates used for interpolation are on the history axis. Assignment rules and
        // the operand itself see the SBML clock of the stage that produced this record.
        self.sink()
            .local_get(BASE)
            .local_get(RECORD)
            .f64_load(byte(h::TIME_SHIFT))
            .local_tee(frame_shift)
            .f64_store(byte(h::ACTIVE_SHIFT))
            .local_get(0)
            .local_get(frame_shift)
            .f64_sub()
            .local_set(0);
        for symbol in used {
            match symbol {
                Symbol::State(i) => {
                    self.sink()
                        .local_get(1)
                        .local_get(x)
                        .local_get(BASE)
                        .local_get(RECORD)
                        .i32_load(byte(h::COEFF))
                        .i32_add()
                        .i32_const((i * 8) as i32)
                        .i32_add()
                        .local_get(RECORD)
                        .i32_load(byte(h::ORDER));
                    self.call_defined(INTERPOLATE);
                    self.sink().f64_store(slot(i));
                }
                Symbol::Param(i) => {
                    self.sink()
                        .local_get(2)
                        .local_get(BASE)
                        .local_get(RECORD)
                        .i32_load(byte(h::PARAMS))
                        .i32_add()
                        .f64_load(slot(i))
                        .f64_store(slot(i));
                }
                _ => {}
            }
        }
        for rule in rules {
            self.let_(rule.target, &rule.expr);
        }
        self.f64(operand);
        self.sink().else_().f64_const(f64c(f64::NAN)).end().end();
        self.sink()
            .local_get(BASE)
            .local_get(caller_shift)
            .f64_store(byte(h::ACTIVE_SHIFT));
        self.sink()
            .local_get(BASE)
            .local_get(DEPTH)
            .i32_store(byte(h::DEPTH));
        Ok(())
    }

    /// `(query, header) -> record address`, zero if no published predictor is available.
    fn locate(&mut self) {
        const BASE: u32 = 1;
        const LO: u32 = 2;
        const HI: u32 = 3;
        const MID: u32 = 4;
        const REC: u32 = 5;
        const TABLE: u32 = 6;
        self.integers = 5;
        self.sink()
            .local_get(0)
            .local_get(BASE)
            .f64_load(byte(h::COMMITTED))
            .f64_gt()
            .if_(BlockType::Empty)
            .local_get(BASE)
            .local_get(BASE)
            .f64_load(byte(h::MAX_QUERY))
            .local_get(0)
            .f64_max()
            .f64_store(byte(h::MAX_QUERY))
            .end();
        self.address_field(BASE, h::RECORDS);
        self.sink()
            .local_set(TABLE)
            .local_get(BASE)
            .i32_load(byte(h::N_RECORDS))
            .local_set(HI);
        // Upper bound. If there is an exact match use the last one, after simultaneous changes.
        self.sink()
            .block(BlockType::Empty)
            .loop_(BlockType::Empty)
            .local_get(LO)
            .local_get(HI)
            .i32_ge_u()
            .br_if(1)
            .local_get(HI)
            .local_get(LO)
            .i32_sub()
            .i32_const(1)
            .i32_shr_u()
            .local_get(LO)
            .i32_add()
            .local_tee(MID)
            .i32_const(h::RECORD_BYTES as i32)
            .i32_mul()
            .local_get(TABLE)
            .i32_add()
            .f64_load(byte(h::END))
            .local_get(0)
            .f64_le()
            .if_(BlockType::Empty)
            .local_get(MID)
            .i32_const(1)
            .i32_add()
            .local_set(LO)
            .else_()
            .local_get(MID)
            .local_set(HI)
            .end()
            .br(0)
            .end()
            .end();
        self.sink()
            .local_get(LO)
            .i32_const(h::RECORD_BYTES as i32)
            .i32_mul()
            .local_get(TABLE)
            .i32_add()
            .local_set(REC)
            .local_get(LO)
            .if_(BlockType::Empty)
            .local_get(REC)
            .i32_const(h::RECORD_BYTES as i32)
            .i32_sub()
            .f64_load(byte(h::END))
            .local_get(0)
            .f64_eq()
            .if_(BlockType::Empty)
            .local_get(REC)
            .i32_const(h::RECORD_BYTES as i32)
            .i32_sub()
            .local_set(REC)
            .local_get(LO)
            .i32_const(1)
            .i32_sub()
            .local_set(LO)
            .end()
            .end()
            .local_get(LO)
            .local_get(BASE)
            .i32_load(byte(h::N_RECORDS))
            .i32_ge_u()
            .if_(BlockType::Empty);
        self.fail(BASE, h::MISSING_RECORD);
        // Extrapolate the latest polynomial as a predictor. The engine must validate
        // overlapping trial steps before publishing them.
        self.sink()
            .local_get(LO)
            .i32_eqz()
            .if_(BlockType::Empty)
            .i32_const(0)
            .return_()
            .end()
            .local_get(REC)
            .i32_const(h::RECORD_BYTES as i32)
            .i32_sub()
            .local_set(REC)
            .end()
            .local_get(REC);
    }

    /// `(normalized time, coefficient address for this state, order) -> state value`.
    fn interpolate_state(&mut self, ny: u32) {
        let value = self.local();
        // Start at the highest coefficient. For a point record (order zero) the time
        // coordinate may be NaN, so the loop must perform exactly zero multiplications.
        self.sink()
            .local_get(1)
            .local_get(2)
            .i32_const((ny * 8) as i32)
            .i32_mul()
            .i32_add()
            .local_tee(1)
            .f64_load(slot(0))
            .local_set(value)
            .block(BlockType::Empty)
            .loop_(BlockType::Empty)
            .local_get(2)
            .i32_eqz()
            .br_if(1)
            .local_get(value)
            .local_get(0)
            .f64_mul()
            .local_get(1)
            .i32_const((ny * 8) as i32)
            .i32_sub()
            .local_tee(1)
            .f64_load(slot(0))
            .f64_add()
            .local_set(value)
            .local_get(2)
            .i32_const(1)
            .i32_sub()
            .local_set(2)
            .br(0)
            .end()
            .end()
            .local_get(value);
    }
}
