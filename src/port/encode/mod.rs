//! Back-ends: typed IR → target bytecode, validating (plan §4.1 / §9 step 3).

pub mod cs2;
pub mod interface;

pub use cs2::{ProcAllocator, encode_ir, ir_to_asm, lower_to_compiled, validate_ir};
