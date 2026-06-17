use crate::cache::FlatCache;
use crate::cache_bail as bail;
use crate::constants::DEFAULT_CACHE_TAR;
use crate::error::{Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use tar::Archive;

pub fn default_cache_dir() -> PathBuf {
    PathBuf::from("../eval/cache-flat/cache")
}

pub fn default_tar_path() -> PathBuf {
    PathBuf::from("..").join(DEFAULT_CACHE_TAR)
}

pub fn open_cache(cache_dir: Option<&Path>) -> Result<FlatCache> {
    let path = cache_dir.map_or_else(default_cache_dir, Path::to_path_buf);
    FlatCache::open(path)
}

pub fn ensure_archive_complete(cache_dir: &Path, tar_path: &Path, archive: u32) -> Result<()> {
    let index_path = cache_dir.join(format!("255/{archive}.dat"));
    if !index_path.is_file() {
        let mut needed = BTreeSet::new();
        needed.insert(format!("cache/255/{archive}.dat"));
        extract_entries(
            tar_path,
            cache_dir
                .parent()
                .context("cache directory has no parent")?,
            &needed,
        )?;
    }

    let cache = FlatCache::open(cache_dir)?;
    let index = cache.archive_index(archive)?;
    let mut needed = BTreeSet::new();
    for group in &index.group_id {
        let path = cache_dir.join(format!("{archive}/{group}.dat"));
        if !path.is_file() {
            needed.insert(format!("cache/{archive}/{group}.dat"));
        }
    }
    if !needed.is_empty() {
        extract_entries(
            tar_path,
            cache_dir
                .parent()
                .context("cache directory has no parent")?,
            &needed,
        )?;
    }
    Ok(())
}

pub fn ensure_archive_groups(
    cache_dir: &Path,
    tar_path: &Path,
    archive: u32,
    groups: &[u32],
) -> Result<()> {
    let index_path = cache_dir.join(format!("255/{archive}.dat"));
    if !index_path.is_file() {
        let mut needed = BTreeSet::new();
        needed.insert(format!("cache/255/{archive}.dat"));
        extract_entries(
            tar_path,
            cache_dir
                .parent()
                .context("cache directory has no parent")?,
            &needed,
        )?;
    }

    let mut needed = BTreeSet::new();
    for group in groups {
        let path = cache_dir.join(format!("{archive}/{group}.dat"));
        if !path.is_file() {
            needed.insert(format!("cache/{archive}/{group}.dat"));
        }
    }
    if !needed.is_empty() {
        extract_entries(
            tar_path,
            cache_dir
                .parent()
                .context("cache directory has no parent")?,
            &needed,
        )?;
    }
    Ok(())
}

fn extract_entries(tar_path: &Path, output_root: &Path, wanted: &BTreeSet<String>) -> Result<()> {
    if !tar_path.is_file() {
        bail!("cache tar not found: {}", tar_path.display());
    }
    fs::create_dir_all(output_root)
        .with_context(|| format!("failed creating {}", output_root.display()))?;

    let file = fs::File::open(tar_path)
        .with_context(|| format!("failed opening {}", tar_path.display()))?;
    let mut archive = Archive::new(file);
    let mut remaining = wanted.clone();

    for entry in archive
        .entries()
        .context("failed to enumerate tar entries")?
    {
        let mut entry = entry.context("failed reading tar entry")?;
        if remaining.is_empty() {
            break;
        }
        let path = entry
            .path()
            .context("tar entry missing path")?
            .to_string_lossy()
            .to_string();
        if !remaining.contains(&path) {
            continue;
        }
        let out_path = output_root.join(&path);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed creating {}", parent.display()))?;
        }
        entry
            .unpack(&out_path)
            .with_context(|| format!("failed extracting {}", out_path.display()))?;
        remaining.remove(&path);
    }

    if !remaining.is_empty() {
        bail!("tar missing {} requested entries", remaining.len());
    }
    Ok(())
}
