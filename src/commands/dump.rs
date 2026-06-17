//! Cache-dump commands for the overlay workflow: `dump-raw-flat`, `dump-refs`,
//! `dump-configs`, and `prepare-overlay` (which orchestrates the first three
//! plus the dependency files into a semantic tree + manifest).

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;

use crate::cache::FlatCache;
use crate::cli::context::CommandContext;
use crate::cli::shared::print_json;
use crate::constants::{
    ARCHIVE_CLIENTSCRIPTS, ARCHIVE_CONFIG, ARCHIVE_ENUM_CONFIG, ARCHIVE_INTERFACES,
    ARCHIVE_LOC_CONFIG, ARCHIVE_NPC_CONFIG, ARCHIVE_OBJ_CONFIG, ARCHIVE_SEQ_CONFIG,
    ARCHIVE_SPOT_CONFIG, ARCHIVE_STRUCT_CONFIG,
};
use crate::dep_tree::ResolverContext;
use crate::fixture::ensure_archive_complete;

/// Config archives dumped together for the overlay refs / config-text dumps.
const CONFIG_ARCHIVES: [u32; 8] = [
    ARCHIVE_CONFIG,
    ARCHIVE_ENUM_CONFIG,
    ARCHIVE_OBJ_CONFIG,
    ARCHIVE_NPC_CONFIG,
    ARCHIVE_LOC_CONFIG,
    ARCHIVE_SEQ_CONFIG,
    ARCHIVE_SPOT_CONFIG,
    ARCHIVE_STRUCT_CONFIG,
];

/// Options for `dump-raw-flat`.
#[derive(Clone, Debug)]
pub struct DumpRawFlatOpts {
    pub out_dir: PathBuf,
    pub archives: Option<String>,
}

/// Options for `dump-refs`.
#[derive(Clone, Debug)]
pub struct DumpRefsOpts {
    pub out_dir: PathBuf,
}

/// Options for `dump-configs`.
#[derive(Clone, Debug)]
pub struct DumpConfigsOpts {
    pub out_dir: PathBuf,
}

/// Options for `prepare-overlay`.
#[derive(Clone, Debug)]
pub struct PrepareOverlayOpts {
    /// Semantic root (e.g. cache/rs3-cache/947-all).
    pub out_dir: PathBuf,
    pub archives: Option<String>,
}

fn dump_raw_flat(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: &Path,
    archives: Option<&str>,
) -> Result<()> {
    let archive_filter: Option<Vec<u32>> = archives.map(|s| {
        s.split(',')
            .filter_map(|p| p.trim().parse::<u32>().ok())
            .collect()
    });

    // Ensure all archives are extracted from tar if needed
    if tar_path.is_file() {
        let archives_to_ensure = if let Some(filter) = archive_filter.as_deref() {
            filter.to_vec()
        } else {
            crate::dump::discover_archives(cache)?
        };
        for id in archives_to_ensure {
            crate::fixture::ensure_archive_complete(cache.root(), tar_path, id)?;
        }
    }

    let stats = crate::dump::dump_raw_flat(cache, out_dir, archive_filter.as_deref())?;

    eprintln!(
        "Dumped {} archives, {} groups, {} bytes in {} ms",
        stats.archives, stats.groups_copied, stats.total_bytes, stats.elapsed_ms
    );
    Ok(())
}

fn dump_refs(cache: &FlatCache, tar_path: &Path, out_dir: &Path, build: u32) -> Result<()> {
    for archive in CONFIG_ARCHIVES {
        ensure_archive_complete(cache.root(), tar_path, archive)?;
    }
    let cache = FlatCache::open(cache.root())?;
    let graph = crate::config_refs::build_config_ref_graph(&cache, build)?;
    crate::config_refs::write_refs_json(&graph, out_dir)?;
    Ok(())
}

fn dump_deps(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    build: u32,
    subbuild: u32,
) -> Result<()> {
    let ctx = ResolverContext::load_lazy(cache, tar_path, data_dir, build, subbuild)?;
    let _ = crate::overlay_deps::write_dependency_files(&ctx, out_dir)?;
    Ok(())
}

fn dump_configs(cache: &FlatCache, tar_path: &Path, out_dir: &Path, build: u32) -> Result<()> {
    for archive in CONFIG_ARCHIVES {
        ensure_archive_complete(cache.root(), tar_path, archive)?;
    }
    let cache2 = FlatCache::open(cache.root())?;
    crate::config_dump::dump_config_texts(&cache2, out_dir, build)?;
    Ok(())
}

/// `dump-raw-flat`.
pub fn run_raw_flat(ctx: &CommandContext, opts: DumpRawFlatOpts) -> Result<()> {
    let DumpRawFlatOpts { out_dir, archives } = opts;
    dump_raw_flat(ctx.cache(), ctx.tar_path(), &out_dir, archives.as_deref())
}

/// `dump-refs`.
pub fn run_refs(ctx: &CommandContext, opts: DumpRefsOpts) -> Result<()> {
    let DumpRefsOpts { out_dir } = opts;
    dump_refs(ctx.cache(), ctx.tar_path(), &out_dir, ctx.build())
}

/// `dump-configs`.
pub fn run_configs(ctx: &CommandContext, opts: DumpConfigsOpts) -> Result<()> {
    let DumpConfigsOpts { out_dir } = opts;
    dump_configs(ctx.cache(), ctx.tar_path(), &out_dir, ctx.build())
}

/// `prepare-overlay` — raw-flat + refs + deps + manifest.
pub fn run_prepare_overlay(ctx: &CommandContext, opts: PrepareOverlayOpts) -> Result<()> {
    let PrepareOverlayOpts {
        out_dir: semantic_root,
        archives,
    } = opts;
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let data_dir = ctx.data_dir();
    let build = ctx.build();
    let subbuild = ctx.subbuild();
    let archives = archives.as_deref();
    let semantic_root = semantic_root.as_path();

    let mut commands_run = Vec::new();
    let raw_flat_started = Instant::now();

    let raw_flat_dir = semantic_root.join("raw-flat");
    dump_raw_flat(cache, tar_path, &raw_flat_dir, archives)?;
    let raw_flat_elapsed = raw_flat_started.elapsed();
    commands_run.push("dump-raw-flat".to_string());

    let refs_dir = semantic_root.join("refs");
    let deps_dir = semantic_root.join("deps");
    for archive in [
        ARCHIVE_CONFIG,
        ARCHIVE_ENUM_CONFIG,
        ARCHIVE_OBJ_CONFIG,
        ARCHIVE_NPC_CONFIG,
        ARCHIVE_LOC_CONFIG,
        ARCHIVE_SEQ_CONFIG,
        ARCHIVE_SPOT_CONFIG,
        ARCHIVE_STRUCT_CONFIG,
        ARCHIVE_INTERFACES,
        ARCHIVE_CLIENTSCRIPTS,
    ] {
        ensure_archive_complete(cache.root(), tar_path, archive)?;
    }
    let cache_root = cache.root().to_path_buf();
    let (refs_result, deps_result) = rayon::join(
        || -> Result<std::time::Duration> {
            let started = Instant::now();
            let refs_cache = FlatCache::open(&cache_root)?;
            dump_refs(&refs_cache, tar_path, &refs_dir, build)?;
            Ok(started.elapsed())
        },
        || -> Result<std::time::Duration> {
            let started = Instant::now();
            let deps_cache = FlatCache::open(&cache_root)?;
            dump_deps(&deps_cache, tar_path, data_dir, &deps_dir, build, subbuild)?;
            Ok(started.elapsed())
        },
    );
    let refs_elapsed = refs_result?;
    let deps_elapsed = deps_result?;
    commands_run.push("dump-refs".to_string());
    commands_run.push("dump-deps".to_string());

    let cache_fingerprint = crate::overlay_manifest::cache_fingerprint(cache);
    let artifacts = vec![
        crate::overlay_manifest::artifact_record("raw-flat", semantic_root)?,
        crate::overlay_manifest::artifact_record("refs", semantic_root)?,
        crate::overlay_manifest::artifact_record("deps", semantic_root)?,
    ];

    let manifest = crate::overlay_manifest::Rs3CacheManifest {
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        build,
        subbuild,
        cache_dir: cache
            .root()
            .canonicalize()
            .unwrap_or_else(|_| cache.root().to_path_buf())
            .to_string_lossy()
            .into_owned(),
        cache_fingerprint,
        semantic_root: semantic_root
            .canonicalize()
            .unwrap_or_else(|_| semantic_root.to_path_buf())
            .to_string_lossy()
            .into_owned(),
        artifacts,
        commands_run,
        finished_at: crate::overlay_manifest::now_rfc3339(),
        skip_config_dumps: Some(true),
    };

    let manifest_path = crate::overlay_manifest::write_manifest(&manifest, semantic_root)?;
    eprintln!(
        "Prepared overlay semantic tree at {} (manifest: {})",
        semantic_root.display(),
        manifest_path.display()
    );
    eprintln!(
        "prepare-overlay timing: raw-flat={}ms refs={}ms deps={}ms",
        raw_flat_elapsed.as_millis(),
        refs_elapsed.as_millis(),
        deps_elapsed.as_millis(),
    );
    let _ = print_json(&manifest);
    Ok(())
}
