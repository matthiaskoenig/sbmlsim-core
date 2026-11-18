//! Delay-only history and model adaptation. Ordinary integrators carry a zero-sized mode.

use std::cell::RefCell;
use std::rc::Rc;

use sbmlsim_abi::{ModelDescription, ModelSizes, SimulationStats, history as abi};

use crate::model::ModelFns;
use crate::solver::Integrator;
use crate::{EngineError, Metadata, Plan, RecordedValues, Scratch};

pub(crate) const HISTORY_FLAG: i32 = -1006;

pub(crate) trait Mode: Clone + Default {
    const ENABLED: bool;
    fn shared(&self) -> Option<&Rc<RefCell<History>>> {
        None
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct NoHistory;
impl Mode for NoHistory {
    const ENABLED: bool = false;
}

#[derive(Clone, Default)]
pub(crate) struct Enabled(pub(crate) Rc<RefCell<History>>);
impl Mode for Enabled {
    const ENABLED: bool = true;
    fn shared(&self) -> Option<&Rc<RefCell<History>>> {
        Some(&self.0)
    }
}

enum Solvers {
    Ode(Option<Integrator<false, Enabled>>),
    Dae(Option<Integrator<true, Enabled>>),
}

pub(crate) struct Cache {
    state: Enabled,
    solvers: Solvers,
}

impl Cache {
    pub(crate) fn new(desc: &ModelDescription) -> Self {
        Self {
            state: Enabled(Rc::new(RefCell::new(History::new(desc)))),
            solvers: if desc.n_algebraic == 0 {
                Solvers::Ode(None)
            } else {
                Solvers::Dae(None)
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        model: &dyn ModelFns,
        desc: &ModelDescription,
        metadata: &mut Metadata,
        plan: &Plan,
        values: &[f64],
        scratch: &mut Scratch,
        prefix: &dyn Fn(&[String]) -> usize,
        stats: &mut SimulationStats,
    ) -> Result<RecordedValues, EngineError> {
        self.state.0.borrow_mut().reset();
        let model = DelayedModel {
            base: model,
            history: self.state.0.clone(),
        };
        match &mut self.solvers {
            Solvers::Ode(cache) => crate::run_model(
                &model,
                desc,
                metadata,
                cache,
                plan,
                values,
                scratch,
                prefix,
                stats,
                self.state.clone(),
            ),
            Solvers::Dae(cache) => crate::run_model(
                &model,
                desc,
                metadata,
                cache,
                plan,
                values,
                scratch,
                prefix,
                stats,
                self.state.clone(),
            ),
        }
    }

    pub(crate) fn initial_values(
        &mut self,
        model: &dyn ModelFns,
        desc: &ModelDescription,
        scratch: &mut Scratch,
    ) -> Result<Vec<f64>, EngineError> {
        self.state.0.borrow_mut().reset();
        let model = DelayedModel {
            base: model,
            history: self.state.0.clone(),
        };
        crate::initial_values(&model, desc, &[], scratch)?;
        if desc.n_algebraic > 0 {
            crate::initial::consistent(&model, desc, &[], scratch, &Default::default())?;
        }
        let n = desc.selections.len();
        let mut out = vec![0.0; n];
        model.selections(
            crate::INIT_TIME,
            &scratch.y,
            &scratch.p,
            &vec![1; n],
            &mut out,
        );
        model.check_error()?;
        Ok(out)
    }
}

#[derive(Clone)]
pub(crate) struct Record {
    pub end: f64,
    pub h: f64,
    pub order: usize,
    pub coefficients: usize,
    pub parameters: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct Checkpoint {
    records: usize,
    bytes: usize,
    table: usize,
    capacity: usize,
    parameters: usize,
}

#[derive(Default)]
pub(crate) struct History {
    pub data: Vec<u8>,
    pub records: Vec<Record>,
    dirty: Vec<(usize, usize)>,
    table: usize,
    table_capacity: usize,
    n: usize,
    np: usize,
    frames: usize,
    sites: usize,
    jumps: Vec<f64>,
    jump_orders: Vec<u32>,
    target_orders: Vec<u32>,
    tracked_parameters: Vec<usize>,
    pub committed: f64,
    pub stage_shift: f64,
    pub start: f64,
    pub stop: f64,
    pub max_step: f64,
    pub step_budget: u64,
    pub settings: sbmlsim_abi::IntegratorSettings,
    pub pending: Option<(f64, i32)>,
    pub new_segment: bool,
    /// The largest age a history read can reach, from the description; `None` leaves the reach
    /// of a lag unbounded, so every accepted step is kept ([`History::prune`]).
    max_delay: Option<f64>,
    /// The end of the fixed vectors of the arena, where the record table and the records of a
    /// compaction start again.
    fixed_end: usize,
    /// The length of the arena after the last compaction; the next one waits for twice it.
    compacted_at: usize,
    /// The largest age a probe published, which never raises the floor of a compaction above
    /// the bound of the description.
    max_age: f64,
    /// Reused buffers of the steps and of the compaction.
    staging: Vec<u8>,
    probe: Vec<f64>,
    queries: Vec<f64>,
    current_parameters: Vec<f64>,
    parameter_offset: usize,
    initialized: bool,
    error: Option<EngineError>,
}

impl History {
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            records: self.records.len(),
            bytes: self.data.len(),
            table: self.table,
            capacity: self.table_capacity,
            parameters: self.parameter_offset,
        }
    }

    pub fn restore(&mut self, checkpoint: Checkpoint) {
        self.records.truncate(checkpoint.records);
        self.data.truncate(checkpoint.bytes);
        self.table = checkpoint.table;
        self.table_capacity = checkpoint.capacity;
        self.parameter_offset = checkpoint.parameters;
        self.put_word(abi::RECORDS, self.table);
        self.put_word(abi::N_RECORDS, self.records.len());
        self.dirty.clear();
    }
    fn new(desc: &ModelDescription) -> Self {
        Self {
            n: desc.states.len(),
            np: desc.parameters.len() + desc.n_constants as usize,
            frames: (desc.n_delays as usize + 2).min(128),
            sites: desc.n_delays as usize,
            max_delay: desc.max_delay.filter(|d| d.is_finite() && *d >= 0.0),
            tracked_parameters: desc
                .parameters
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.settable.then_some(i))
                .collect(),
            ..Self::default()
        }
    }

    fn reset(&mut self) {
        self.data.clear();
        self.records.clear();
        self.jumps.clear();
        self.jump_orders.clear();
        self.target_orders.clear();
        self.dirty.clear();
        self.current_parameters.clear();
        self.parameter_offset = 0;
        self.initialized = false;
        self.error = None;
        self.pending = None;
        self.committed = 0.0;
        self.stage_shift = 0.0;
        self.fixed_end = 0;
        self.compacted_at = 0;
        self.max_age = 0.0;
    }

    pub fn fail(&mut self, error: EngineError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    fn allocate(&mut self, len: usize) -> Result<usize, EngineError> {
        let start = self.data.len().next_multiple_of(8);
        let end = start
            .checked_add(len)
            .filter(|&n| n <= 1 << 30)
            .ok_or_else(|| self.failure(&self.overflow("exceeds 1 GiB")))?;
        self.data
            .try_reserve(end - self.data.len())
            .map_err(|_| self.failure(&self.overflow("does not fit into memory")))?;
        self.data.resize(end, 0);
        self.dirty.push((start, len));
        Ok(start)
    }

    /// What a user can do about an arena which outgrew its room. A bounded model keeps only
    /// twice its longest lag, so only a lag of the state or of the time grows without end.
    fn overflow(&self, what: &str) -> String {
        let records = self.records.len();
        let kept = if self.max_delay.is_some() {
            "The history keeps twice the longest delay of the model: shorten the delays, raise \
             the tolerances so the integrator takes longer steps, or reduce the states"
        } else {
            "A delay whose duration depends on the state or on the time has no bound, so every \
             accepted step is kept: give the delays constant durations, raise the tolerances so \
             the integrator takes longer steps, or shorten the simulation"
        };
        format!("the delay history of {records} accepted steps {what}. {kept}")
    }

    pub fn failure(&self, message: &str) -> EngineError {
        EngineError::Integration {
            flag: HISTORY_FLAG,
            time: self.committed,
            message: message.into(),
        }
    }

    pub fn word(&self, at: u32) -> u32 {
        u32::from_le_bytes(self.data[at as usize..at as usize + 4].try_into().unwrap())
    }
    fn put_word(&mut self, at: u32, value: usize) {
        self.data[at as usize..at as usize + 4].copy_from_slice(&(value as u32).to_le_bytes());
    }
    pub fn number(&self, at: u32) -> f64 {
        f64::from_le_bytes(self.data[at as usize..at as usize + 8].try_into().unwrap())
    }
    pub fn put_number(&mut self, at: u32, value: f64) {
        self.data[at as usize..at as usize + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn store_values(&mut self, values: &[f64]) -> Result<usize, EngineError> {
        let at = self.allocate(values.len() * 8)?;
        for (out, value) in self.data[at..at + values.len() * 8]
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .zip(values)
        {
            out.copy_from_slice(&value.to_le_bytes());
        }
        Ok(at)
    }

    fn initialize(&mut self, y: &[f64], p: &[f64], fixed: &[u8]) -> Result<(), EngineError> {
        self.allocate(abi::HEADER_BYTES as usize)?;
        let y = self.store_values(y)?;
        let p = self.store_values(p)?;
        let f = self.allocate(fixed.len())?;
        self.data[f..f + fixed.len()].copy_from_slice(fixed);
        let scratch = self.allocate(self.frames * (self.n + self.np) * 8)?;
        self.put_word(abi::DEFAULT_Y, y);
        self.put_word(abi::DEFAULT_P, p);
        self.put_word(abi::FIXED, f);
        self.put_word(abi::SCRATCH, scratch);
        self.put_word(abi::CAPACITY, self.frames);
        let targets = self.store_values(&vec![f64::INFINITY; abi::ROOTS_PER_QUERY * self.sites])?;
        let queries = self.store_values(&vec![f64::NAN; self.sites])?;
        self.put_word(abi::TARGETS, targets);
        self.put_word(abi::QUERIES, queries);
        self.fixed_end = self.data.len();
        self.table_capacity = 128;
        self.table = self.allocate(self.table_capacity * abi::RECORD_BYTES as usize)?;
        self.put_word(abi::RECORDS, self.table);
        self.put_number(abi::COMMITTED, 0.0);
        self.put_number(abi::HISTORY_START, 0.0);
        self.put_number(abi::MAX_QUERY, f64::NEG_INFINITY);
        self.put_number(abi::MIN_LAG, f64::INFINITY);
        self.initialized = true;
        Ok(())
    }

    pub fn sync(&mut self, model: &dyn ModelFns, p: &mut [f64]) -> Result<(), EngineError> {
        let address = self.data.as_mut_ptr() as usize;
        let base = model.set_history(&self.data, &self.dirty, address)?;
        if let Some(slot) = p.last_mut() {
            *slot = base as f64;
        }
        self.dirty.clear();
        Ok(())
    }

    pub fn diagnostics(&mut self, model: &dyn ModelFns) -> Result<(), EngineError> {
        if !self.initialized {
            return Ok(());
        }
        let mut header = [0u8; abi::HEADER_BYTES as usize];
        header.copy_from_slice(&self.data[..abi::HEADER_BYTES as usize]);
        if model.read_history_status(&mut header)? {
            for (offset, length) in [(abi::STATUS, 4), (abi::MAX_QUERY, 8), (abi::MIN_LAG, 8)] {
                let range = offset as usize..offset as usize + length;
                self.data[range.clone()].copy_from_slice(&header[range]);
            }
        }
        match self.word(abi::STATUS) {
            abi::INVALID_LAG => Err(self.failure("delay duration must be finite and nonnegative")),
            abi::DEPTH_EXCEEDED => {
                Err(self.failure("delay evaluation exceeds the history frame limit"))
            }
            abi::PROJECTION_FAILED => {
                Err(self.failure("historical algebraic constraints cannot be resolved"))
            }
            _ => Ok(()),
        }
    }

    pub fn clear_diagnostics(&mut self) {
        self.put_word(abi::STATUS, 0);
        self.put_number(abi::MAX_QUERY, f64::NEG_INFINITY);
        self.put_number(abi::MIN_LAG, f64::INFINITY);
    }

    /// Writes the descriptor of a record into the slot of `self.records.len()` of the table.
    fn publish(&mut self, record: &Record, shift: f64) {
        let at = self.table + self.records.len() * abi::RECORD_BYTES as usize;
        self.data[at..at + 8].copy_from_slice(&record.end.to_le_bytes());
        self.data[at + 8..at + 16].copy_from_slice(&record.h.to_le_bytes());
        self.data[at + 16..at + 20].copy_from_slice(&(record.order as u32).to_le_bytes());
        self.data[at + 20..at + 24].copy_from_slice(&(record.coefficients as u32).to_le_bytes());
        self.data[at + 24..at + 28].copy_from_slice(&(record.parameters as u32).to_le_bytes());
        self.data[at + 32..at + 40].copy_from_slice(&shift.to_le_bytes());
        self.dirty.push((at, abi::RECORD_BYTES as usize));
    }

    fn store_bytes(&mut self, bytes: &[u8]) -> Result<usize, EngineError> {
        let at = self.allocate(bytes.len())?;
        self.data[at..at + bytes.len()].copy_from_slice(bytes);
        Ok(at)
    }

    /// Drops the records and the jumps no history read can reach any more, and rebuilds the
    /// arena without them.
    ///
    /// A model whose every lag of a `delay` is a finite nonnegative constant cannot read further
    /// back than `max_delay` of its description, and the floor keeps twice it, so no read of a
    /// retained step, of a rejected trial of one, or of the neighbour of a jump of one can fall
    /// below the floor. Without that bound (a lag of the state or of the time) nothing is
    /// dropped. The arena has to double before it is rebuilt again, so the work of the
    /// compactions of a run stays proportional to the steps of the run.
    pub(crate) fn prune(&mut self) -> Result<(), EngineError> {
        /// The newest records, which the interpolation of the committed time and the overlap of
        /// the next trial need whatever the bound is.
        const RETAIN: usize = 8;
        /// Below it a run keeps its history whole: a compaction cannot pay for itself.
        const THRESHOLD: usize = 1 << 20;
        let Some(max_delay) = self.max_delay else {
            return Ok(());
        };
        if !self.initialized
            || self.records.len() <= RETAIN
            || self.data.len() < THRESHOLD.max(2 * self.compacted_at)
        {
            return Ok(());
        }
        let reach = max_delay.max(self.max_age);
        let floor = self.committed + self.stage_shift - 2.0 * reach;
        let drop = self
            .records
            .partition_point(|r| r.end < floor)
            .min(self.records.len() - RETAIN);
        if drop == 0 {
            // Nothing is reclaimable yet; waiting for twice the arena keeps the check amortized.
            self.compacted_at = self.data.len();
            return Ok(());
        }
        let dropped = self.jumps.partition_point(|&jump| jump < floor);
        self.jumps.drain(..dropped);
        self.jump_orders.drain(..dropped);
        self.compact(drop)
    }

    fn compact(&mut self, drop: usize) -> Result<(), EngineError> {
        let np = self.np * 8;
        let records = std::mem::take(&mut self.records);
        let mut staging = std::mem::take(&mut self.staging);
        staging.clear();
        // The live blocks leave the arena before it is rebuilt over them. A record which shares
        // the parameters of the one before it keeps sharing them.
        let mut plan = Vec::with_capacity(records.len() - drop);
        let mut previous = usize::MAX;
        for (i, record) in records.iter().enumerate().skip(drop) {
            let shift = self.number(
                (self.table + i * abi::RECORD_BYTES as usize + abi::TIME_SHIFT as usize) as u32,
            );
            let bytes = (record.order + 1) * self.n * 8;
            let coefficients = staging.len();
            staging.extend_from_slice(&self.data[record.coefficients..record.coefficients + bytes]);
            let parameters = if record.parameters == previous {
                usize::MAX
            } else {
                previous = record.parameters;
                let at = staging.len();
                staging.extend_from_slice(&self.data[record.parameters..record.parameters + np]);
                at
            };
            plan.push((record.clone(), shift, coefficients, parameters));
        }
        self.data.truncate(self.fixed_end);
        self.table_capacity = plan.len().next_power_of_two().max(128);
        self.table = self.allocate(self.table_capacity * abi::RECORD_BYTES as usize)?;
        let mut parameters = 0;
        for (mut record, shift, coefficients, at) in plan {
            if at != usize::MAX {
                parameters = self.store_bytes(&staging[at..at + np])?;
            }
            let bytes = (record.order + 1) * self.n * 8;
            record.coefficients = self.store_bytes(&staging[coefficients..coefficients + bytes])?;
            record.parameters = parameters;
            self.publish(&record, shift);
            self.records.push(record);
        }
        self.parameter_offset = parameters;
        self.put_word(abi::RECORDS, self.table);
        self.put_word(abi::N_RECORDS, self.records.len());
        let start = self.records.first().map_or(f64::INFINITY, |r| r.end);
        self.put_number(abi::HISTORY_START, start);
        // The arena moved, so the model reads all of it again.
        self.dirty.clear();
        self.dirty.push((0, self.data.len()));
        self.compacted_at = self.data.len();
        self.staging = staging;
        Ok(())
    }

    fn parameters(&mut self, p: &[f64]) -> Result<usize, EngineError> {
        let same = self.same_parameters(p);
        if !same {
            self.parameter_offset = self.store_values(p)?;
            self.current_parameters.clear();
            self.current_parameters.extend_from_slice(p);
        }
        Ok(self.parameter_offset)
    }

    pub fn append(
        &mut self,
        end: f64,
        h: f64,
        order: usize,
        coefficients: &[f64],
        p: &[f64],
    ) -> Result<(), EngineError> {
        let end = end + self.stage_shift;
        self.put_number(abi::HISTORY_START, self.number(abi::HISTORY_START).min(end));
        while self.records.last().is_some_and(|r| r.end > end) {
            self.records.pop();
        }
        if self
            .records
            .last()
            .is_some_and(|r| r.end == end && r.h == 0.0 && h == 0.0)
        {
            self.records.pop();
        }
        if self.records.len() == self.table_capacity {
            let old = self.table;
            self.table_capacity *= 2;
            self.table = self.allocate(self.table_capacity * abi::RECORD_BYTES as usize)?;
            let bytes = self.records.len() * abi::RECORD_BYTES as usize;
            self.data.copy_within(old..old + bytes, self.table);
            self.put_word(abi::RECORDS, self.table);
        }
        let parameters = self.parameters(p)?;
        let coefficients = self.store_values(coefficients)?;
        let record = Record {
            end,
            h,
            order,
            coefficients,
            parameters,
        };
        self.publish(&record, self.stage_shift);
        self.records.push(record);
        self.put_word(abi::N_RECORDS, self.records.len());
        Ok(())
    }

    pub fn point(&mut self, t: f64, y: &[f64], p: &[f64]) -> Result<(), EngineError> {
        self.truncate(t)?;
        self.append(t, 0.0, 0, &y[..self.n], p)?;
        self.committed = t;
        self.put_number(abi::COMMITTED, t + self.stage_shift);
        Ok(())
    }

    pub fn mark_jump(&mut self, t: f64, order: u32) {
        // Higher-order discontinuities need propagation only through the largest method order.
        if order > 12 {
            return;
        }
        let axis = t + self.stage_shift;
        match self.jumps.binary_search_by(|value| value.total_cmp(&axis)) {
            Ok(i) => self.jump_orders[i] = self.jump_orders[i].min(order),
            Err(i) => {
                self.jumps.insert(i, axis);
                self.jump_orders.insert(i, order);
            }
        }
    }

    pub fn mark_roots(&mut self, roots: &[i32], first: usize) {
        let original_end = roots
            .len()
            .saturating_sub(abi::ROOTS_PER_QUERY * self.sites);
        let t = self.committed;
        if roots[first.min(original_end)..original_end]
            .iter()
            .any(|&flag| flag != 0)
        {
            self.mark_jump(t, 0);
        }
        let order = roots[original_end..]
            .iter()
            .zip(&self.target_orders)
            .filter_map(|(&flag, &order)| {
                (flag != 0 && order != u32::MAX).then_some(order.saturating_add(1))
            })
            .min();
        if let Some(order) = order {
            self.mark_jump(t, order);
        }
    }

    pub fn set_jump_targets(
        &mut self,
        model: &dyn ModelFns,
        p: &mut [f64],
    ) -> Result<(), EngineError> {
        let at = self.word(abi::QUERIES) as usize;
        let mut queries = std::mem::take(&mut self.queries);
        queries.clear();
        queries.extend(
            self.data[at..at + self.sites * 8]
                .as_chunks::<8>()
                .0
                .iter()
                .map(|b| f64::from_le_bytes(*b)),
        );
        model.read_history_queries(at, &mut queries)?;
        let axis = self.committed + self.stage_shift;
        for &query in queries.iter().filter(|q| q.is_finite()) {
            self.max_age = self.max_age.max(axis - query);
        }
        self.target_orders.clear();
        let at = self.word(abi::TARGETS) as usize;
        for (i, query) in queries.iter().enumerate() {
            let tol = 32.0 * f64::EPSILON * query.abs().max(1.0);
            let lower = self.jumps.partition_point(|&jump| jump < query - tol);
            let upper = self.jumps.partition_point(|&jump| jump <= query + tol);
            // A query can reverse before reaching either neighbor. Retain the
            // just-crossed epoch separately so its return crossing stays armed.
            let indices = if query.is_finite() {
                [
                    lower.checked_sub(1),
                    Some(upper),
                    (lower < upper).then_some(lower),
                ]
            } else {
                [None; abi::ROOTS_PER_QUERY]
            };
            for (j, index) in indices.into_iter().enumerate() {
                let target = index
                    .and_then(|k| self.jumps.get(k))
                    .copied()
                    .unwrap_or(if j == 0 {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    });
                self.target_orders.push(
                    index
                        .and_then(|k| self.jump_orders.get(k))
                        .copied()
                        .unwrap_or(u32::MAX),
                );
                let slot = abi::ROOTS_PER_QUERY * i + j;
                self.data[at + slot * 8..at + (slot + 1) * 8]
                    .copy_from_slice(&target.to_le_bytes());
            }
        }
        self.dirty.push((at, abi::ROOTS_PER_QUERY * self.sites * 8));
        self.queries = queries;
        self.sync(model, p)
    }

    pub fn rebase(&mut self, start: f64) {
        self.stage_shift += self.committed - start;
        self.committed = start;
        self.pending = None;
        self.put_number(abi::COMMITTED, start + self.stage_shift);
        self.put_number(abi::STAGE_SHIFT, self.stage_shift);
        self.put_number(abi::ACTIVE_SHIFT, self.stage_shift);
    }

    fn merge_fixed(&mut self, y: &[f64], p: &[f64], fixed: &[u8]) {
        let np = self.np - 1;
        let fixed_at = self.word(abi::FIXED) as usize;
        for (i, &set) in fixed.iter().enumerate() {
            if set == 0 {
                continue;
            }
            let at = if i < np {
                self.word(abi::DEFAULT_P) as usize + i * 8
            } else {
                self.word(abi::DEFAULT_Y) as usize + (i - np) * 8
            };
            let value = if i < np { p[i] } else { y[i - np] };
            self.data[at..at + 8].copy_from_slice(&value.to_le_bytes());
            self.data[fixed_at + i] = set;
            self.dirty.push((at, 8));
            self.dirty.push((fixed_at + i, 1));
        }
    }

    fn same_parameters(&self, p: &[f64]) -> bool {
        self.current_parameters.len() == p.len()
            && self
                .tracked_parameters
                .iter()
                .all(|&i| self.current_parameters[i].to_bits() == p[i].to_bits())
    }

    fn truncate(&mut self, t: f64) -> Result<(), EngineError> {
        let t = t + self.stage_shift;
        let at = self.records.partition_point(|r| r.end <= t);
        let Some(record) = self.records.get(at).cloned() else {
            return Ok(());
        };
        if record.h == 0.0 {
            self.records.truncate(at);
            return Ok(());
        }
        let x = (t - record.end) / record.h;
        let mut coefficients = vec![0.0; (record.order + 1) * self.n];
        for j in 0..=record.order {
            let mut binomial = 1.0;
            for k in j..=record.order {
                if k > j {
                    binomial *= k as f64 / (k - j) as f64;
                }
                let scale = binomial * x.powi((k - j) as i32);
                for i in 0..self.n {
                    let at = record.coefficients + (k * self.n + i) * 8;
                    coefficients[j * self.n + i] +=
                        scale * f64::from_le_bytes(self.data[at..at + 8].try_into().unwrap());
                }
            }
        }
        let p: Vec<f64> = self.data[record.parameters..record.parameters + self.np * 8]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| f64::from_le_bytes(*b))
            .collect();
        self.records.truncate(at);
        self.append(
            t - self.stage_shift,
            record.h,
            record.order,
            &coefficients,
            &p,
        )
    }

    fn changed(&mut self, t: f64, y: &[f64], p: &[f64]) -> bool {
        if !self.same_parameters(p) || self.records.is_empty() {
            return true;
        }
        let mut expected = std::mem::take(&mut self.probe);
        expected.clear();
        expected.resize(self.n, 0.0);
        let changed = match self.evaluate(t, &mut expected) {
            Err(_) => true,
            Ok(()) => expected.iter().zip(y).any(|(a, b)| {
                (a - b).abs() > 8.0 * f64::EPSILON * (a.abs() + b.abs()).max(f64::MIN_POSITIVE)
            }),
        };
        self.probe = expected;
        changed
    }

    pub fn evaluate(&self, t: f64, out: &mut [f64]) -> Result<(), EngineError> {
        let t = t + self.stage_shift;
        let pos = self.records.partition_point(|r| r.end < t);
        let mut pos = pos.min(self.records.len().saturating_sub(1));
        while pos + 1 < self.records.len() && self.records[pos + 1].end == t {
            pos += 1;
        }
        let record = self
            .records
            .get(pos)
            .ok_or_else(|| self.failure("delay history has no initial point"))?;
        let x = if record.h == 0.0 {
            0.0
        } else {
            (t - record.end) / record.h
        };
        for (i, value) in out.iter_mut().take(self.n).enumerate() {
            *value = 0.0;
            for k in (0..=record.order).rev() {
                let at = record.coefficients + (k * self.n + i) * 8;
                *value = *value * x + f64::from_le_bytes(self.data[at..at + 8].try_into().unwrap());
            }
        }
        Ok(())
    }
}

struct DelayedModel<'a> {
    base: &'a dyn ModelFns,
    history: Rc<RefCell<History>>,
}

impl DelayedModel<'_> {
    fn save(&self, t: f64, y: &[f64], p: &mut [f64], force: bool, order: u32) {
        let mut history = self.history.borrow_mut();
        if let Err(error) = history.diagnostics(self.base) {
            history.fail(error);
            return;
        }
        let changed = force || history.changed(t, y, p);
        if changed {
            history.mark_jump(t, order);
        }
        let result = if changed {
            history.point(t, y, p)
        } else {
            Ok(())
        }
        .and_then(|()| history.sync(self.base, p));
        if let Err(error) = result {
            history.fail(error);
        }
    }
}

impl ModelFns for DelayedModel<'_> {
    fn set_history(
        &self,
        data: &[u8],
        updates: &[(usize, usize)],
        address: usize,
    ) -> Result<u32, EngineError> {
        self.base.set_history(data, updates, address)
    }
    fn read_history_status(&self, data: &mut [u8]) -> Result<bool, EngineError> {
        self.base.read_history_status(data)
    }
    fn read_history_queries(&self, offset: usize, out: &mut [f64]) -> Result<bool, EngineError> {
        self.base.read_history_queries(offset, out)
    }
    fn check_error(&self) -> Result<(), EngineError> {
        self.base.check_error()?;
        let mut history = self.history.borrow_mut();
        if let Some(error) = &history.error {
            return Err(error.clone());
        }
        history.diagnostics(self.base)
    }
    fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
        self.base.defaults(p, y);
    }
    fn init(&self, t: f64, p: &mut [f64], y: &mut [f64], fixed: &[u8]) {
        {
            let mut history = self.history.borrow_mut();
            let result = if history.initialized {
                history.merge_fixed(y, p, fixed);
                Ok(())
            } else {
                history.initialize(y, p, fixed)
            }
            .and_then(|()| history.sync(self.base, p));
            if let Err(error) = result {
                history.fail(error);
                return;
            }
        }
        self.base.init(t, p, y, fixed);
        self.save(t, y, p, true, 1);
    }
    fn rhs(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        self.base.rhs(t, y, p, out);
    }
    fn jac(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        self.base.jac(t, y, p, out);
    }
    fn roots(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        self.base.roots(t, y, p, out);
    }
    fn trigger(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> bool {
        self.base.trigger(i, t, y, p)
    }
    fn priority(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        self.base.priority(i, t, y, p)
    }
    fn delay(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        self.base.delay(i, t, y, p)
    }
    fn event_values(&self, i: u32, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        self.base.event_values(i, t, y, p, out);
    }
    fn event_apply(&self, i: u32, t: f64, values: &[f64], y: &mut [f64], p: &mut [f64]) {
        self.base.event_apply(i, t, values, y, p);
        self.save(t, y, p, true, 0);
    }
    fn selections(&self, t: f64, y: &[f64], p: &[f64], mask: &[u8], out: &mut [f64]) {
        self.base.selections(t, y, p, mask, out);
    }
    fn assign(&self, t: f64, y: &mut [f64], p: &mut [f64]) {
        self.base.assign(t, y, p);
        self.save(t, y, p, false, 0);
    }
    fn sizes(&self) -> ModelSizes {
        self.base.sizes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A history of one state and one parameter slot (the address of the header).
    fn history(max_delay: Option<f64>) -> History {
        let mut history = History {
            n: 1,
            np: 1,
            frames: 3,
            sites: 1,
            max_delay,
            ..History::default()
        };
        history.initialize(&[0.0], &[0.0], &[0]).unwrap();
        history
    }

    #[test]
    fn a_bounded_lag_keeps_the_arena_flat_and_its_window_exact() {
        const STEPS: usize = 40_000;
        const DT: f64 = 1e-3;
        let mut sizes = Vec::new();
        for max_delay in [Some(1.0), None] {
            let mut history = history(max_delay);
            for k in 1..=STEPS {
                let t = k as f64 * DT;
                // the linear record through (t - DT, t - DT) and (t, t), so every
                // interpolation of the window returns its own time
                history.append(t, DT, 1, &[t, DT], &[0.0]).unwrap();
                history.committed = t;
                history.prune().unwrap();
            }
            let end = STEPS as f64 * DT;
            let mut out = [0.0];
            for query in [end, end - 0.5, end - 1.0] {
                history.evaluate(query, &mut out).unwrap();
                assert!((out[0] - query).abs() < 1e-9, "{query} gave {}", out[0]);
            }
            if max_delay.is_some() {
                // twice the bound of the lag, so the window of the records is 2 of time
                assert!(
                    history.records.len() < STEPS / 4,
                    "{}",
                    history.records.len()
                );
                assert!(history.data.len() < 1 << 21, "{} bytes", history.data.len());
            } else {
                assert_eq!(history.records.len(), STEPS);
            }
            sizes.push(history.data.len());
        }
        assert!(sizes[0] * 4 < sizes[1], "{sizes:?}");
    }

    #[test]
    fn a_compaction_keeps_the_published_table_and_the_arena_consistent() {
        let mut history = history(Some(0.0));
        for k in 1..=20_000 {
            let t = k as f64 * 1e-3;
            history.append(t, 1e-3, 1, &[t, 1e-3], &[0.0]).unwrap();
            history.committed = t;
            history.prune().unwrap();
        }
        // every descriptor of the table is the record the engine holds
        assert_eq!(history.word(abi::N_RECORDS) as usize, history.records.len());
        let table = history.word(abi::RECORDS) as usize;
        for (i, record) in history.records.iter().enumerate() {
            let at = (table + i * abi::RECORD_BYTES as usize) as u32;
            assert_eq!(history.number(at + abi::END), record.end);
            assert_eq!(history.number(at + abi::H), record.h);
            assert_eq!(history.word(at + abi::COEFF) as usize, record.coefficients);
            assert_eq!(history.word(at + abi::PARAMS) as usize, record.parameters);
            assert!(record.coefficients + (record.order + 1) * history.n * 8 <= history.data.len());
            assert!(record.parameters + history.np * 8 <= history.data.len());
        }
        assert_eq!(
            history.number(abi::HISTORY_START),
            history.records[0].end,
            "the floor of the retained records is the start of the published history"
        );
    }
}
