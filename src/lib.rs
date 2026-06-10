#![deny(
    clippy::dbg_macro,
    clippy::todo,
    clippy::unimplemented,
    clippy::unwrap_used
)]
// Global lint policy: binary cache parsing requires frequent integer
// casts between u8/u16/u32/i32/usize. Many false positives in pedantic.
#![allow(
    // Binary data parsing: necessary truncations from u32→u16, i32→usize, etc.
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::checked_conversions,
    // Config parsers naturally have long functions (many field reads)
    clippy::too_many_lines,
    // Module naming follows domain convention (e.g. src/transpile/ast.rs → ast mod)
    clippy::module_name_repetitions,
    // Several config parsers have identical arm bodies for similar struct fields
    clippy::match_same_arms,
    // Not all public APIs need must_use (e.g. emit functions)
    clippy::must_use_candidate,
    // Extensive use of Option::map plus unwrap_or; map_unwrap_or is less readable here
    clippy::map_unwrap_or,
    // Documentation gaps are tracked separately; public API is still evolving
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    // Builder patterns: new() returns Self without must_use
    clippy::return_self_not_must_use,
    // Not all const-eligible functions benefit from const (e.g. those returning Strings)
    clippy::missing_const_for_fn,
    // Match arms using Option::map often clearer than if-let chains
    clippy::option_if_let_else,
    // items_after_statements: helper fns after main logic in some functions
    clippy::items_after_statements,
    // unused_self: some methods take &self for symmetry with sibling methods
    clippy::unused_self,
)]

pub mod animator;
pub mod audio;
pub mod cache;
pub mod cli;
pub mod collision;
pub mod config;
pub mod config_dump;
pub mod config_refs;
pub mod constants;
pub mod cutscene2d;
pub mod dep_tree;
pub mod dump;
pub mod error;
pub mod fixture;
pub mod interface;
pub mod interface_codec;
pub mod js5;
pub mod map;
pub mod migrate;
pub mod model;
pub mod overlay_deps;
pub mod overlay_manifest;
pub mod overlay_plan;
pub mod packet;
pub mod parallel;
pub mod script;
pub mod script_transpile;
pub mod transpile;
pub mod validate;
pub mod vars;
pub mod vfx;
