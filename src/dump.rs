use crate::cache::FlatCache;
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct RawFlatStats {
    pub archives: usize,
    pub groups_copied: u64,
    pub total_bytes: u64,
    pub elapsed_ms: u64,
}

/// Scan a directory for `*.dat` files whose integer stem matches `filter`.
fn discover_ids(dir: &Path, filter: impl Fn(u32) -> bool) -> Result<Vec<u32>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<u32> = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| {
            let entry = e.ok()?;
            let fname = entry.file_name();
            let stem = fname.to_string_lossy();
            let id = stem.strip_suffix(".dat")?.parse::<u32>().ok()?;
            filter(id).then_some(id)
        })
        .collect();
    ids.sort_unstable();
    Ok(ids)
}

/// Discover available archive IDs from the flat cache's `255/` directory.
pub fn discover_archives(cache: &FlatCache) -> Result<Vec<u32>> {
    discover_ids(&cache.root().join("255"), |id| id != 255)
}

/// Discover group IDs for an archive from its flat cache directory.
pub fn discover_groups(cache: &FlatCache, archive: u32) -> Result<Vec<u32>> {
    discover_ids(&cache.root().join(archive.to_string()), |_| true)
}

fn copy_file(src: &Path, dst: &Path) -> Result<u64> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::copy(src, dst).with_context(|| format!("copying {} → {}", src.display(), dst.display()))
}

fn copy_group(
    cache: &FlatCache,
    archive: u32,
    group: u32,
    out_dir: &Path,
    bytes: &AtomicU64,
    count: &AtomicU64,
) -> Result<()> {
    let src = cache.root().join(format!("{archive}/{group}.dat"));
    if !src.is_file() {
        return Ok(());
    }
    let dst = out_dir.join(format!("{archive}/{group}.dat"));
    bytes.fetch_add(copy_file(&src, &dst)?, Ordering::Relaxed);
    count.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// Dump a lossless, byte-perfect flat cache tree.
///
/// Produces `out_dir/255/<archive>.dat` and `out_dir/<archive>/<group>.dat`
/// for every archive and group present in the source cache.
pub fn dump_raw_flat(
    cache: &FlatCache,
    out_dir: &Path,
    archive_filter: Option<&[u32]>,
) -> Result<RawFlatStats> {
    let start = Instant::now();
    let total_bytes = AtomicU64::new(0);
    let total_groups = AtomicU64::new(0);
    let allowed: Option<BTreeSet<u32>> = archive_filter.map(|a| a.iter().copied().collect());

    fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output dir {}", out_dir.display()))?;

    let archives: Vec<u32> = discover_archives(cache)?
        .into_iter()
        .filter(|id| allowed.as_ref().is_none_or(|a| a.contains(id)))
        .collect();

    // Copy archive indices (255/<archive>.dat)
    let index_dir = out_dir.join("255");
    fs::create_dir_all(&index_dir)
        .with_context(|| format!("creating index dir {}", index_dir.display()))?;

    let master = cache.root().join("255/255.dat");
    if master.is_file() {
        copy_file(&master, &index_dir.join("255.dat"))?;
    }

    archives
        .par_iter()
        .try_for_each(|&archive| -> Result<()> {
            let src = cache.root().join(format!("255/{archive}.dat"));
            if src.is_file() {
                total_bytes.fetch_add(
                    copy_file(&src, &index_dir.join(format!("{archive}.dat")))?,
                    Ordering::Relaxed,
                );
            }

            let groups = discover_groups(cache, archive)?;
            if groups.is_empty() {
                return Ok(());
            }

            let archive_dir = out_dir.join(archive.to_string());
            fs::create_dir_all(&archive_dir)
                .with_context(|| format!("creating archive dir {}", archive_dir.display()))?;

            groups.par_iter().try_for_each(|&group| -> Result<()> {
                copy_group(cache, archive, group, out_dir, &total_bytes, &total_groups)
            })?;

            Ok(())
        })?;

    Ok(RawFlatStats {
        archives: archives.len(),
        groups_copied: total_groups.load(Ordering::Relaxed),
        total_bytes: total_bytes.load(Ordering::Relaxed),
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}
