use crate::cache::FlatCache;
use crate::error::{Context, Result};
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct RawFlatStats {
    pub archives: usize,
    pub groups_copied: u64,
    pub total_bytes: u64,
    pub elapsed_ms: u64,
}

const GROUP_COPY_CHUNK_SIZE: usize = 1024;

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

/// Discover available archive IDs from flat cache's `255/` directory.
pub fn discover_archives(cache: &FlatCache) -> Result<Vec<u32>> {
    discover_ids(&cache.root().join("255"), |id| id != 255)
}

/// Discover group IDs for archive from flat cache directory.
pub fn discover_groups(cache: &FlatCache, archive: u32) -> Result<Vec<u32>> {
    discover_ids(&cache.root().join(archive.to_string()), |_| true)
}

fn copy_file(src: &Path, dst: &Path) -> Result<u64> {
    fs::copy(src, dst).with_context(|| format!("copying {} → {}", src.display(), dst.display()))
}

fn copy_file_with_parent(src: &Path, dst: &Path) -> Result<u64> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    copy_file(src, dst)
}

fn copy_group_chunk(src_dir: &Path, dst_dir: &Path, groups: &[u32]) -> Result<(u64, u64)> {
    let mut total_bytes = 0_u64;
    let mut copied = 0_u64;
    for &group in groups {
        let src = src_dir.join(format!("{group}.dat"));
        if !src.is_file() {
            continue;
        }
        let dst = dst_dir.join(format!("{group}.dat"));
        total_bytes += copy_file(&src, &dst)?;
        copied += 1;
    }
    Ok((total_bytes, copied))
}

fn copy_archive_groups(
    cache: &FlatCache,
    archive: u32,
    groups: &[u32],
    archive_dir: &Path,
) -> Result<(u64, u64)> {
    let src_dir = cache.root().join(archive.to_string());
    if groups.len() <= GROUP_COPY_CHUNK_SIZE {
        return copy_group_chunk(&src_dir, archive_dir, groups);
    }

    let chunk_stats = groups
        .par_chunks(GROUP_COPY_CHUNK_SIZE)
        .map(|groups| copy_group_chunk(&src_dir, archive_dir, groups))
        .collect::<Result<Vec<_>>>()?;
    Ok(chunk_stats.into_iter().fold(
        (0_u64, 0_u64),
        |(bytes, count), (chunk_bytes, chunk_count)| (bytes + chunk_bytes, count + chunk_count),
    ))
}

/// Dump lossless, byte-perfect flat cache tree.
///
/// Produces `out_dir/255/<archive>.dat` and `out_dir/<archive>/<group>.dat`
/// for every archive and group present in source cache.
pub fn dump_raw_flat(
    cache: &FlatCache,
    out_dir: &Path,
    archive_filter: Option<&[u32]>,
) -> Result<RawFlatStats> {
    let start = Instant::now();
    let allowed: Option<BTreeSet<u32>> = archive_filter.map(|a| a.iter().copied().collect());

    fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output dir {}", out_dir.display()))?;

    let archives: Vec<u32> = discover_archives(cache)?
        .into_iter()
        .filter(|id| allowed.as_ref().is_none_or(|a| a.contains(id)))
        .collect();
    let index_dir = out_dir.join("255");
    fs::create_dir_all(&index_dir)
        .with_context(|| format!("creating index dir {}", index_dir.display()))?;

    let mut index_bytes = 0_u64;
    let master = cache.root().join("255/255.dat");
    if master.is_file() {
        index_bytes += copy_file_with_parent(&master, &index_dir.join("255.dat"))?;
    }

    let archive_stats = archives
        .par_iter()
        .map(|&archive| -> Result<(u64, u64, u64)> {
            let mut local_index_bytes = 0_u64;
            let src = cache.root().join(format!("255/{archive}.dat"));
            if src.is_file() {
                local_index_bytes +=
                    copy_file_with_parent(&src, &index_dir.join(format!("{archive}.dat")))?;
            }

            let groups = discover_groups(cache, archive)?;
            if groups.is_empty() {
                return Ok((local_index_bytes, 0, 0));
            }

            let archive_dir = out_dir.join(archive.to_string());
            fs::create_dir_all(&archive_dir)
                .with_context(|| format!("creating archive dir {}", archive_dir.display()))?;
            let (group_bytes, groups_copied) =
                copy_archive_groups(cache, archive, &groups, &archive_dir)?;
            Ok((local_index_bytes, group_bytes, groups_copied))
        })
        .collect::<Result<Vec<_>>>()?;

    let (extra_index_bytes, group_bytes, groups_copied) = archive_stats.into_iter().fold(
        (0_u64, 0_u64, 0_u64),
        |(index_total, group_total, group_count),
         (archive_index, archive_groups, archive_count)| {
            (
                index_total + archive_index,
                group_total + archive_groups,
                group_count + archive_count,
            )
        },
    );
    index_bytes += extra_index_bytes;

    Ok(RawFlatStats {
        archives: archives.len(),
        groups_copied,
        total_bytes: index_bytes + group_bytes,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::dump_raw_flat;
    use crate::cache::FlatCache;
    use crate::error::Result;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn dump_raw_flat_respects_archive_filter() -> Result<()> {
        let cache_dir = tempdir()?;
        let out_dir = tempdir()?;

        fs::create_dir_all(cache_dir.path().join("255"))?;
        fs::create_dir_all(cache_dir.path().join("1"))?;
        fs::create_dir_all(cache_dir.path().join("2"))?;

        fs::write(cache_dir.path().join("255/255.dat"), [1_u8, 2, 3])?;
        fs::write(cache_dir.path().join("255/1.dat"), [4_u8, 5])?;
        fs::write(cache_dir.path().join("255/2.dat"), [6_u8])?;
        fs::write(cache_dir.path().join("1/3.dat"), [7_u8, 8, 9, 10])?;
        fs::write(cache_dir.path().join("1/7.dat"), [11_u8])?;
        fs::write(cache_dir.path().join("2/1.dat"), [12_u8, 13])?;

        let cache = FlatCache::open(cache_dir.path())?;
        let stats = dump_raw_flat(&cache, out_dir.path(), Some(&[1]))?;

        assert_eq!(stats.archives, 1);
        assert_eq!(stats.groups_copied, 2);
        assert_eq!(stats.total_bytes, 10);

        assert_eq!(fs::read(out_dir.path().join("255/255.dat"))?, vec![1, 2, 3]);
        assert_eq!(fs::read(out_dir.path().join("255/1.dat"))?, vec![4, 5]);
        assert!(!out_dir.path().join("255/2.dat").exists());
        assert_eq!(fs::read(out_dir.path().join("1/3.dat"))?, vec![7, 8, 9, 10]);
        assert_eq!(fs::read(out_dir.path().join("1/7.dat"))?, vec![11]);
        assert!(!out_dir.path().join("2/1.dat").exists());

        Ok(())
    }
}
