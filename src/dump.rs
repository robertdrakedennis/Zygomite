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

/// `reflink_or_copy` returns `Some(bytes)` when it fell back to a full byte
/// copy and `None` when it cloned, in which case the dest matches the source
/// size.
fn reflink_bytes(src: &Path, outcome: Option<u64>) -> Result<u64> {
    match outcome {
        Some(bytes) => Ok(bytes),
        None => fs::metadata(src)
            .map(|meta| meta.len())
            .with_context(|| format!("sizing {}", src.display())),
    }
}

/// Copy a group `.dat` via a copy-on-write clone when the filesystem supports
/// it (APFS `clonefile`, Linux `FICLONE`), falling back to a full byte copy on
/// cross-volume or non-reflink targets. Cloning a 10GB+ flat cache is metadata
/// only — near-instant and zero extra disk — versus physically rewriting every
/// byte with `fs::copy`.
///
/// `reflink_or_copy` refuses to overwrite an existing destination. A re-run
/// hits that case; we fall back to `fs::copy`, which overwrites in place — as
/// cheap as the original copy and far cheaper than unlink-then-clone. The
/// common fresh-dump path clones directly with no extra syscall per file.
fn copy_file(src: &Path, dst: &Path) -> Result<u64> {
    match reflink_copy::reflink_or_copy(src, dst) {
        Ok(outcome) => reflink_bytes(src, outcome),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => fs::copy(src, dst)
            .with_context(|| format!("overwriting {} → {}", src.display(), dst.display())),
        Err(err) => {
            Err(err).with_context(|| format!("cloning {} → {}", src.display(), dst.display()))
        }
    }
}

fn copy_file_with_parent(src: &Path, dst: &Path) -> Result<u64> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    copy_file(src, dst)
}

/// Clone an entire archive directory tree in a single reflink syscall when the
/// filesystem supports it. On macOS `clonefile` clones the whole `<archive>/`
/// subtree in one in-kernel call — vastly cheaper than reflinking 80k+ group
/// files one at a time, where per-file syscall overhead, not bytes, dominates.
/// Falls back to a per-file copy where directory-level reflink is unavailable
/// (e.g. Linux, whose reflink is file-granular) or across volumes.
fn clone_tree(src_dir: &Path, dst_dir: &Path) -> Result<()> {
    // One in-kernel clonefile clones the whole subtree on a fresh dest.
    match reflink_copy::reflink(src_dir, dst_dir) {
        Ok(()) => Ok(()),
        // Dest already exists (a re-run, e.g. `--force` rebuild). reflink won't
        // overwrite, so clear the stale mirror and re-clone — far cheaper than a
        // per-file physical re-copy, and it drops files no longer in the source.
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            remove_tree_parallel(dst_dir)?;
            match reflink_copy::reflink(src_dir, dst_dir) {
                Ok(()) => Ok(()),
                Err(_) => copy_tree_fallback(src_dir, dst_dir),
            }
        }
        // Filesystem lacks directory-level reflink (e.g. Linux, whose reflink is
        // file-granular) or cross-volume: per-file path, parallelized.
        Err(_) => copy_tree_fallback(src_dir, dst_dir),
    }
}

/// Remove a directory tree, unlinking files in parallel. `fs::remove_dir_all`
/// unlinks serially, which is the long pole when clearing a stale mirror of an
/// 80k-file archive before a re-clone; spreading the unlinks across the pool
/// keeps a `--force` re-run faster than the original in-place overwrite.
fn remove_tree_parallel(dir: &Path) -> Result<()> {
    let entries: Vec<(std::path::PathBuf, bool)> = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
        .map(|entry| {
            let is_dir = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
            (entry.path(), is_dir)
        })
        .collect();
    entries
        .par_chunks(GROUP_COPY_CHUNK_SIZE)
        .try_for_each(|chunk| -> Result<()> {
            for (path, is_dir) in chunk {
                if *is_dir {
                    remove_tree_parallel(path)?;
                } else {
                    fs::remove_file(path)
                        .with_context(|| format!("removing {}", path.display()))?;
                }
            }
            Ok(())
        })?;
    fs::remove_dir(dir).with_context(|| format!("removing {}", dir.display()))
}

/// Per-file fallback when directory-level reflink is unavailable. Each file
/// still goes through `copy_file`, which itself attempts a file-level reflink
/// before a full byte copy.
fn copy_tree_fallback(src_dir: &Path, dst_dir: &Path) -> Result<()> {
    fs::create_dir_all(dst_dir).with_context(|| format!("creating {}", dst_dir.display()))?;
    let names: Vec<std::ffi::OsString> = fs::read_dir(src_dir)
        .with_context(|| format!("reading {}", src_dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
        .collect();
    names
        .par_chunks(GROUP_COPY_CHUNK_SIZE)
        .try_for_each(|chunk| -> Result<()> {
            for name in chunk {
                let src = src_dir.join(name);
                if src.is_file() {
                    copy_file(&src, &dst_dir.join(name))?;
                }
            }
            Ok(())
        })
}

/// Count and total the byte size of group (`<u32>.dat`) files directly under
/// `dir`, reading sizes in parallel. Used to report dump stats after a tree
/// clone, which (unlike a per-file copy) returns no byte tally itself.
fn dir_stats(dir: &Path) -> (u64, u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (0, 0);
    };
    let entries: Vec<fs::DirEntry> = entries.flatten().collect();
    entries
        .par_iter()
        .filter_map(|entry| {
            let name = entry.file_name();
            let is_group = name
                .to_str()
                .and_then(|stem| stem.strip_suffix(".dat"))
                .is_some_and(|stem| stem.parse::<u32>().is_ok());
            is_group.then(|| entry.metadata().map_or(0, |meta| meta.len()))
        })
        .map(|len| (1_u64, len))
        .reduce(|| (0, 0), |(c1, b1), (c2, b2)| (c1 + c2, b1 + b2))
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
            let src_index = cache.root().join(format!("255/{archive}.dat"));
            if src_index.is_file() {
                local_index_bytes +=
                    copy_file_with_parent(&src_index, &index_dir.join(format!("{archive}.dat")))?;
            }

            let src_dir = cache.root().join(archive.to_string());
            if !src_dir.is_dir() {
                return Ok((local_index_bytes, 0, 0));
            }

            let archive_dir = out_dir.join(archive.to_string());
            clone_tree(&src_dir, &archive_dir)?;
            let (groups_copied, group_bytes) = dir_stats(&src_dir);
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
