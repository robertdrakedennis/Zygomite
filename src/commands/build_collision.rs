//! `build-collision` — build the RS clip-flag collision grid for one map square.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::cache::FlatCache;
use crate::cli::context::CommandContext;
use crate::cli::shared::{print_json, write_text};
use crate::config::parse_loc;
use crate::constants::{
    ARCHIVE_CONFIG, ARCHIVE_LOC_CONFIG, ARCHIVE_MAPSQUARES, CONFIG_GROUP_LOC_LEGACY,
};
use crate::map::decode_map_square_best_effort;

/// Command-specific options for `build-collision`.
#[derive(Clone, Debug)]
pub struct BuildCollisionOpts {
    /// Map-square X coordinate (region X, 0..127).
    pub map_x: u32,
    /// Map-square Z coordinate (region Z, 0..255).
    pub map_z: u32,
    /// Optional path to write per-level flag grids as JSON.
    pub out: Option<PathBuf>,
}

/// Load every loc config and reduce it to its collision-relevant [`LocClip`].
fn load_loc_clips(cache: &FlatCache) -> Result<HashMap<i32, crate::collision::LocClip>> {
    use crate::collision::LocClip;
    let mut clips = HashMap::new();
    if let Ok(loc_index) = cache.archive_index(ARCHIVE_LOC_CONFIG) {
        for group in &loc_index.group_id {
            let files = cache.group_files_with_index(&loc_index, ARCHIVE_LOC_CONFIG, *group)?;
            for (file, data) in files {
                let loc_id = (*group << 8) | file;
                let entry =
                    parse_loc(loc_id, &data).with_context(|| format!("parse_loc id {loc_id}"))?;
                clips.insert(loc_id as i32, LocClip::from_loc_ops(&entry.ops));
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_LOC_LEGACY)? {
        let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_LOC_LEGACY, &payload)?;
        for (id, data) in files {
            let entry = parse_loc(id, &data).with_context(|| format!("parse_loc id {id}"))?;
            clips.insert(id as i32, LocClip::from_loc_ops(&entry.ops));
        }
    }
    Ok(clips)
}

/// Build the clip-flag collision grid for one map square (NXT model).
pub fn run(ctx: &CommandContext, opts: BuildCollisionOpts) -> Result<()> {
    let cache = ctx.cache();
    let build = ctx.build();
    let BuildCollisionOpts { map_x, map_z, out } = opts;

    let group = (map_z << 7) | map_x;
    let index = cache.archive_index(ARCHIVE_MAPSQUARES)?;
    let files = cache
        .group_files_with_index(&index, ARCHIVE_MAPSQUARES, group)
        .with_context(|| format!("load mapsquare group {group} ({map_x}_{map_z})"))?;
    let map = decode_map_square_best_effort(&files, build);
    let clips = load_loc_clips(cache)?;
    let grids =
        crate::collision::build_collision(&map, |id| clips.get(&id).copied().unwrap_or_default());

    #[derive(serde::Serialize)]
    struct LevelSummary {
        level: usize,
        blocked: usize,
    }
    let level_summaries: Vec<LevelSummary> = grids
        .iter()
        .enumerate()
        .map(|(level, g)| LevelSummary {
            level,
            blocked: g.nonzero_count(),
        })
        .collect();

    if let Some(path) = out.as_deref() {
        #[derive(serde::Serialize)]
        struct LevelDump {
            level: usize,
            blocked: usize,
            flags: Vec<Vec<i32>>,
        }
        #[derive(serde::Serialize)]
        struct FullDump {
            build: u32,
            #[serde(rename = "mapX")]
            map_x: u32,
            #[serde(rename = "mapZ")]
            map_z: u32,
            size: usize,
            levels: Vec<LevelDump>,
        }
        let dump = FullDump {
            build,
            map_x,
            map_z,
            size: crate::collision::SQUARE_SIZE,
            levels: grids
                .iter()
                .enumerate()
                .map(|(level, g)| LevelDump {
                    level,
                    blocked: g.nonzero_count(),
                    flags: g.to_rows(),
                })
                .collect(),
        };
        write_text(path, &serde_json::to_string(&dump)?)?;
        eprintln!("build-collision: wrote grids to {}", path.display());
    }

    #[derive(serde::Serialize)]
    struct Summary {
        build: u32,
        #[serde(rename = "mapX")]
        map_x: u32,
        #[serde(rename = "mapZ")]
        map_z: u32,
        size: usize,
        #[serde(rename = "locCount")]
        loc_count: usize,
        levels: Vec<LevelSummary>,
    }
    print_json(&Summary {
        build,
        map_x,
        map_z,
        size: crate::collision::SQUARE_SIZE,
        loc_count: map.locs.len(),
        levels: level_summaries,
    })
}
