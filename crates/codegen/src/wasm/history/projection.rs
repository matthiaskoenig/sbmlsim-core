//! Reconstruct algebraic unknowns at the model time of a prehistory query.
use super::*;

impl Body<'_> {
    pub(super) fn project_history(
        &mut self,
        model: &Model,
        operand: &Expr,
        base: u32,
        fixed: u32,
    ) -> Result<(), CompileError> {
        if model.n_algebraic == 0 {
            return Ok(());
        }
        let indices = algebraic_dependencies(model, operand);
        if indices.is_empty() {
            return Ok(());
        }
        let residuals: Vec<_> = indices.iter().map(|&i| &model.rates[i as usize]).collect();
        let assignments = live_assignments(&model.initial, residuals.iter().copied());
        let evaluate = |body: &mut Self, values: &[u32]| {
            for a in &assignments {
                body.if_not_fixed(fixed, fixed_index(model, a.target));
                body.store(a.target, &a.expr);
                body.end();
            }
            for (expr, &out) in residuals.iter().zip(values) {
                body.f64(expr);
                body.sink().local_set(out);
            }
        };

        let n = indices.len();
        // Before allocating the dense matrix or emitting its elimination, compare
        // lower bounds for locals and instructions with the existing guest limits.
        // Each elimination update needs at least ten encoded bytes.
        let locals = n.saturating_mul(n).saturating_add(n.saturating_mul(4));
        let updates = n.saturating_sub(1).saturating_pow(3) / 3;
        let limits = self.module.limits;
        if locals.saturating_add(self.locals()) > limits.locals
            || updates.saturating_mul(10).saturating_add(self.size()) > limits.bytes
        {
            return Err(CompileError::Unsupported {
                feature: format!(
                    "historical algebraic projection of {n} unknowns exceeds the guest function budget of {} bytes or {} locals",
                    limits.bytes, limits.locals
                ),
                element: "delay".into(),
            });
        }
        let mut vector = || (0..n).map(|_| self.local()).collect::<Vec<_>>();
        let x = vector();
        let rhs = vector();
        let trial = vector();
        let scales = vector();
        let matrix: Vec<_> = (0..n).map(|_| vector()).collect();
        let norm = self.local();
        let step = self.local();
        let temp = self.local();
        let iterations = self.local();

        self.sink().block(BlockType::Empty).loop_(BlockType::Empty);
        evaluate(self, &rhs);
        for (k, &i) in indices.iter().enumerate() {
            self.sink().local_get(1).f64_load(slot(i)).local_set(x[k]);
        }
        // Perturb one unknown at a time and reuse the representable increment.
        for (col, &i) in indices.iter().enumerate() {
            self.sink()
                .local_get(1)
                .local_get(x[col])
                .local_get(x[col])
                .f64_abs()
                .f64_const(f64c(1.))
                .f64_max()
                .f64_const(f64c(f64::EPSILON.sqrt()))
                .f64_mul()
                .f64_add()
                .local_tee(temp)
                .f64_store(slot(i))
                .local_get(temp)
                .local_get(x[col])
                .f64_sub()
                .local_set(step);
            evaluate(self, &trial);
            if !self.within_limits() {
                return Err(self.above_the_limits("delay", "historical algebraic projection: "));
            }
            for row in 0..n {
                self.sink()
                    .local_get(trial[row])
                    .local_get(rhs[row])
                    .f64_sub()
                    .local_get(step)
                    .f64_div()
                    .local_set(matrix[row][col]);
            }
            self.sink()
                .local_get(1)
                .local_get(x[col])
                .f64_store(slot(i));
        }
        // Scale equations by their response to a relative change of each unknown.
        // The residual norm then measures a dimensionless correction.
        self.sink().f64_const(f64c(0.)).local_set(norm);
        for row in 0..n {
            self.sink().f64_const(f64c(f64::MIN_POSITIVE));
            for (&a, &value) in matrix[row].iter().zip(&x) {
                self.sink()
                    .local_get(a)
                    .f64_abs()
                    .local_get(value)
                    .f64_abs()
                    .f64_const(f64c(1.))
                    .f64_max()
                    .f64_mul()
                    .f64_max();
            }
            self.sink()
                .local_tee(scales[row])
                .f64_const(f64c(f64::INFINITY))
                .f64_lt()
                .i32_eqz()
                .if_(BlockType::Empty);
            self.fail(base, h::PROJECTION_FAILED);
            self.sink().br(2).end();
            for &a in &matrix[row] {
                self.sink()
                    .local_get(a)
                    .local_get(scales[row])
                    .f64_div()
                    .local_set(a);
            }
            self.sink()
                .local_get(rhs[row])
                .f64_neg()
                .local_get(scales[row])
                .f64_div()
                .local_tee(rhs[row])
                .f64_abs()
                .local_get(norm)
                .f64_max()
                .local_set(norm);
        }
        self.sink()
            .local_get(norm)
            .f64_const(f64c(1e-11))
            .f64_le()
            .br_if(1);
        self.sink()
            .local_get(iterations)
            .f64_const(f64c(1.))
            .f64_add()
            .local_tee(iterations)
            .f64_const(f64c(32.))
            .f64_gt()
            .if_(BlockType::Empty);
        self.fail(base, h::PROJECTION_FAILED);
        self.sink().br(2).end();

        self.solve_projection_step(&matrix, &rhs, temp);
        self.sink()
            .f64_const(f64c(1.))
            .local_set(step)
            .block(BlockType::Empty)
            .loop_(BlockType::Empty);
        for (row, &i) in indices.iter().enumerate() {
            self.sink()
                .local_get(1)
                .local_get(x[row])
                .local_get(step)
                .local_get(rhs[row])
                .f64_mul()
                .f64_add()
                .f64_store(slot(i));
        }
        evaluate(self, &trial);
        self.sink().f64_const(f64c(0.));
        for row in 0..n {
            self.sink()
                .local_get(trial[row])
                .local_get(scales[row])
                .f64_div()
                .f64_abs()
                .f64_max();
        }
        self.sink()
            .local_get(norm)
            .f64_lt()
            .br_if(1)
            .local_get(step)
            .f64_const(f64c(0.5))
            .f64_mul()
            .local_tee(step)
            .f64_const(f64c(1. / 4096.))
            .f64_ge()
            .br_if(0);
        self.fail(base, h::PROJECTION_FAILED);
        self.sink().br(3).end().end().br(0).end().end();
        Ok(())
    }

    /// Solve the row-scaled linear system in place with partial pivoting.
    fn solve_projection_step(&mut self, matrix: &[Vec<u32>], rhs: &[u32], temp: u32) {
        let n = rhs.len();
        for col in 0..n {
            for row in col + 1..n {
                self.sink()
                    .local_get(matrix[row][col])
                    .f64_abs()
                    .local_get(matrix[col][col])
                    .f64_abs()
                    .f64_gt()
                    .if_(BlockType::Empty);
                for (&a, &b) in matrix[row][col..]
                    .iter()
                    .zip(&matrix[col][col..])
                    .chain(std::iter::once((&rhs[row], &rhs[col])))
                {
                    self.sink()
                        .local_get(a)
                        .local_get(b)
                        .local_set(a)
                        .local_set(b);
                }
                self.end();
            }
            for row in col + 1..n {
                self.sink()
                    .local_get(matrix[row][col])
                    .local_get(matrix[col][col])
                    .f64_div()
                    .local_set(temp);
                for (&a, &b) in matrix[row][col + 1..]
                    .iter()
                    .zip(&matrix[col][col + 1..])
                    .chain(std::iter::once((&rhs[row], &rhs[col])))
                {
                    self.sink()
                        .local_get(a)
                        .local_get(temp)
                        .local_get(b)
                        .f64_mul()
                        .f64_sub()
                        .local_set(a);
                }
            }
        }
        for row in (0..n).rev() {
            self.sink().local_get(rhs[row]);
            for (&a, &v) in matrix[row][row + 1..].iter().zip(&rhs[row + 1..]) {
                self.sink().local_get(a).local_get(v).f64_mul().f64_sub();
            }
            self.sink()
                .local_get(matrix[row][row])
                .f64_div()
                .local_set(rhs[row]);
        }
    }
}

/// Follow equations matched to the unknowns read at this time, including reads
/// through initial assignments. The IR appends matched unknowns and equations in
/// the same order after the differential states.
fn algebraic_dependencies(model: &Model, operand: &Expr) -> Vec<u32> {
    let first = model.states.len() as u32 - model.n_algebraic;
    let mut indices = HashSet::new();
    let mut expressions = vec![operand];
    while let Some(expr) = expressions.pop() {
        let mut needed = HashSet::new();
        reads(expr, &mut needed);
        for assignment in closure(&model.initial, expr) {
            reads(&assignment.expr, &mut needed);
        }
        for symbol in needed {
            if let Symbol::State(i) = symbol
                && i >= first
                && indices.insert(i)
            {
                expressions.push(&model.rates[i as usize]);
            }
        }
    }
    let mut indices: Vec<_> = indices.into_iter().collect();
    indices.sort_unstable();
    indices
}
