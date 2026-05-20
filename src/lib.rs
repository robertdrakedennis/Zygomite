#![deny(
    clippy::dbg_macro,
    clippy::todo,
    clippy::unimplemented,
    clippy::unwrap_used
)]
#![allow(
    clippy::checked_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::items_after_statements,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::option_if_let_else,
    clippy::too_many_lines
)]

pub mod animator;
pub mod audio;
pub mod cache;
pub mod cli;
pub mod config;
pub mod constants;
pub mod cutscene2d;
pub mod fixture;
pub mod interface;
pub mod js5;
pub mod map;
pub mod model;
pub mod packet;
pub mod script;
pub mod vars;
pub mod vfx;
