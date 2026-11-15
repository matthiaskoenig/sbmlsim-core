//! The meaning of every operator of the model representation and its native evaluation, which
//! live in `sbmlsim_ir::eval` (the one implementation the emitter, the initial values of the
//! description and the constant folding of the optimizer share).

pub(crate) use sbmlsim_ir::eval::{
    HALF_PI, QUOTIENT_HIGH, QUOTIENT_INVALID, QUOTIENT_LOW, eval, is_boolean,
};
