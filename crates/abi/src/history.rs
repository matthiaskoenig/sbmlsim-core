//! Relocatable linear-memory history for models with MathML delay. All offsets are bytes
//! from the header. Integers are little-endian u32, times and coefficients are f64.

/// Name of the optional model.wasm custom section identifying this history contract.
pub const SECTION: &str = "sbmlsim.history";
/// First little-endian word in the section. The second is the number of delay operands.
pub const VERSION: u32 = 1;
/// Maximum supported historical nesting, including evaluation of initial assignments.
pub const MAX_FRAMES: usize = 128;
/// Lower, upper, and just-crossed discontinuity roots for each historical query.
pub const ROOTS_PER_QUERY: usize = 3;

/// Offset of the contiguous record descriptors.
pub const RECORDS: u32 = 0;
/// Number of published records, sorted by end time.
pub const N_RECORDS: u32 = 4;
/// Offset of pre-initialization state values.
pub const DEFAULT_Y: u32 = 8;
/// Offset of pre-initialization parameters including constant slots.
pub const DEFAULT_P: u32 = 12;
/// Offset of fixed entity bytes, parameters then states.
pub const FIXED: u32 = 16;
/// Offset of frames, each holding the state and parameter vectors.
pub const SCRATCH: u32 = 20;
/// Current number of active frames.
pub const DEPTH: u32 = 24;
/// Maximum number of frames.
pub const CAPACITY: u32 = 28;
/// Status word written by a failing history lookup.
pub const STATUS: u32 = 32;
/// Last accepted time (f64).
pub const COMMITTED: u32 = 40;
/// Largest query beyond accepted history (f64).
pub const MAX_QUERY: u32 = 48;
/// Smallest positive lag evaluated during this trial (f64, initially infinity).
pub const MIN_LAG: u32 = 56;
/// History-axis time minus model time in the current simulation stage (f64).
pub const STAGE_SHIFT: u32 = 64;
/// History-axis time minus model time in the active evaluation frame (f64).
pub const ACTIVE_SHIFT: u32 = 72;
/// Offset of discontinuity targets (lower, upper, and just crossed) per probe.
pub const TARGETS: u32 = 80;
/// Offset of the history-axis query per probe; inactive probes write NaN.
pub const QUERIES: u32 = 84;
/// Earliest time on the history axis with a published trajectory (f64).
pub const HISTORY_START: u32 = 88;
/// Header length.
pub const HEADER_BYTES: u32 = 96;
/// Record end time (f64).
pub const END: u32 = 0;
/// Time scale of the polynomial (f64).
pub const H: u32 = 8;
/// Polynomial order.
pub const ORDER: u32 = 16;
/// Offset of the coefficients in increasing power, one state vector per power.
pub const COEFF: u32 = 20;
/// Offset of the parameter vector for this record.
pub const PARAMS: u32 = 24;
/// History-axis time minus model time when this record was produced (f64).
pub const TIME_SHIFT: u32 = 32;
/// Record descriptor length.
pub const RECORD_BYTES: u32 = 40;
/// Negative or nonfinite lag.
pub const INVALID_LAG: u32 = 1;
/// No free scratch frame.
pub const DEPTH_EXCEEDED: u32 = 2;
/// Requested time exceeds the published records.
pub const MISSING_RECORD: u32 = 3;
/// Historical algebraic constraints could not be resolved.
pub const PROJECTION_FAILED: u32 = 4;
