//! `verify-map-archive` — verify all mapsquare groups decode from the raw-flat
//! map archive.

use std::time::Instant;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::cli::context::CommandContext;
use crate::constants::ARCHIVE_MAPSQUARES;
use crate::map::decode_map_square;

/// Verify every mapsquare group decodes against the command's build.
pub fn run(ctx: &CommandContext) -> Result<()> {
    let cache = ctx.cache();
    let build = ctx.build();
    let started = Instant::now();
    let index = cache.archive_index(ARCHIVE_MAPSQUARES)?;
    let count = index.group_id.len();
    index
        .group_id
        .par_iter()
        .try_for_each(|group| -> Result<()> {
            let files = cache.group_files_with_index(&index, ARCHIVE_MAPSQUARES, *group)?;
            let square_x = group & 0b111_1111;
            let square_z = group >> 7;
            decode_map_square(&files, build).with_context(|| {
                format!("decode mapsquare group {group} ({square_x}_{square_z})")
            })?;
            Ok(())
        })?;
    eprintln!(
        "verify-map-archive: decoded {} mapsquare group(s) in {}ms",
        count,
        started.elapsed().as_millis()
    );
    Ok(())
}
