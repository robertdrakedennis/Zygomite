//! Per-command handlers, one (or a small cohesive group) per module.
//!
//! Each module exposes a `run(ctx: &CommandContext, opts: …Opts) -> Result<()>`
//! entry point (or a small set of `run_<verb>` entries for multi-verb
//! commands). The clap surface + dispatch lives in [`crate::cli`]; these modules
//! own the command logic. Helpers private to a single command live with it;
//! helpers shared across commands live in [`crate::cli::shared`].

pub mod assemble;
pub mod audio;
pub mod build_collision;
pub mod config_extract;
pub mod cs2;
pub mod dep_tree;
pub mod dump;
pub mod migrate;
pub mod models;
pub mod transpile;
pub mod ts_export;
pub mod unpack;
pub mod validate;
pub mod verify_map;
