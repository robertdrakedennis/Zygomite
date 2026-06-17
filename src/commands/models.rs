//! `models` — decode every (or a sampled set of) RT7 model group, optionally
//! writing each decoded model + a combined JSON dump.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;

use crate::cache::FlatCache;
use crate::cli::context::CommandContext;
use crate::cli::shared::{print_json, write_json};
use crate::constants::ARCHIVE_MODELS_RT7;
use crate::model::Model;

#[derive(Debug, Serialize)]
struct ModelsSummary {
    groups_parsed: usize,
    parse_errors: usize,
}

/// Options for `models`.
#[derive(Clone, Debug, Default)]
pub struct ModelsOpts {
    pub out_file: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub sample_only: bool,
}

/// `models` — decode model groups.
pub fn run(ctx: &CommandContext, opts: ModelsOpts) -> Result<()> {
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let build = ctx.build();
    let ModelsOpts {
        out_file,
        out_dir,
        sample_only,
    } = opts;
    let out_file = out_file.as_deref();
    let out_dir = out_dir.as_deref();

    crate::fixture::ensure_archive_complete(cache.root(), tar_path, ARCHIVE_MODELS_RT7)?;
    let cache = FlatCache::open(cache.root())?;
    let index = cache.archive_index(ARCHIVE_MODELS_RT7)?;
    let available_groups: HashSet<u32> = index.group_id.iter().copied().collect();

    let groups: Vec<u32> = if sample_only {
        let mut sample: Vec<u32> = std::env::var("MODEL_ONLY")
            .ok()
            .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
            .unwrap_or_else(|| {
                let mut s = (0_u32..=100).collect::<Vec<_>>();
                s.extend([1_000, 5_000, 10_000, 50_000, 100_000]);
                if let Some(last) = index.group_id.last() {
                    s.push(*last);
                }
                s
            });
        sample.sort_unstable();
        sample.dedup();
        sample.retain(|group| available_groups.contains(group));
        sample
    } else {
        index.group_id.clone()
    };

    if let Some(path) = out_dir {
        fs::create_dir_all(path).with_context(|| format!("failed creating {}", path.display()))?;
    }

    struct ModelGroupResult {
        parsed_count: usize,
        parse_errors: usize,
        parsed_model: Option<(u32, Model)>,
    }

    let keep_models = out_file.is_some();
    let group_results = groups
        .par_iter()
        .map(|group| -> Result<ModelGroupResult> {
            let files = cache.group_files_with_index(&index, ARCHIVE_MODELS_RT7, *group)?;
            let Some(bytes) = files.get(&0) else {
                return Ok(ModelGroupResult {
                    parsed_count: 0,
                    parse_errors: 0,
                    parsed_model: None,
                });
            };
            match Model::decode(bytes, build) {
                Ok(model) => {
                    if let Some(dir) = out_dir {
                        let model_path = dir.join(format!("model_{group}.json"));
                        write_json(&model_path, &model)?;
                    }
                    Ok(ModelGroupResult {
                        parsed_count: 1,
                        parse_errors: 0,
                        parsed_model: keep_models.then_some((*group, model)),
                    })
                }
                Err(_) => Ok(ModelGroupResult {
                    parsed_count: 0,
                    parse_errors: 1,
                    parsed_model: None,
                }),
            }
        })
        .collect::<Vec<_>>();

    let mut parsed = Vec::new();
    let mut parsed_count = 0_usize;
    let mut parse_errors = 0_usize;
    for result in group_results {
        let result = result?;
        parsed_count += result.parsed_count;
        parse_errors += result.parse_errors;
        if let Some(model) = result.parsed_model {
            parsed.push(model);
        }
    }

    if let Some(path) = out_file {
        write_json(path, &parsed)?;
    }
    print_json(&ModelsSummary {
        groups_parsed: parsed_count,
        parse_errors,
    })
}
