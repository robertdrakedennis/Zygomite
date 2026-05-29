//! Semantic tree manifest for `CacheOverlay` (`prepare-overlay`).

use crate::cache::FlatCache;
use crate::error::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MANIFEST_FILE: &str = ".rs3-cache-manifest.json";

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactRecord {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Rs3CacheManifest {
    pub tool_version: String,
    pub build: u32,
    pub subbuild: u32,
    pub cache_dir: String,
    pub cache_fingerprint: String,
    pub semantic_root: String,
    pub artifacts: Vec<ArtifactRecord>,
    pub commands_run: Vec<String>,
    pub finished_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_config_dumps: Option<bool>,
}

pub fn cache_fingerprint(cache: &FlatCache) -> String {
    let mut hasher = DefaultHasher::new();
    cache.root().to_string_lossy().hash(&mut hasher);
    if let Ok(meta) = fs::metadata(cache.root()) {
        if let Ok(modified) = meta.modified() {
            modified.hash(&mut hasher);
        }
        meta.len().hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn path_mtime_ms(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

fn count_dat_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    // Collect first so subdirectories can be walked in parallel — a flat cache
    // tree has hundreds of thousands of `.dat` files, and `file_type()` reads
    // the kind straight from the directory entry (no per-file `stat` syscall).
    let entries: Vec<fs::DirEntry> = entries.flatten().collect();
    entries
        .par_iter()
        .map(|entry| match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => count_dat_files(&entry.path()),
            Ok(_) => usize::from(
                Path::new(&entry.file_name())
                    .extension()
                    .is_some_and(|ext| ext == "dat"),
            ),
            Err(_) => 0,
        })
        .sum()
}

pub fn artifact_record(relative: &str, root: &Path) -> Result<ArtifactRecord> {
    let path = root.join(relative);
    let file_count = if path.is_dir() {
        Some(count_dat_files(&path))
    } else {
        None
    };
    Ok(ArtifactRecord {
        path: relative.to_string(),
        mtime_ms: path_mtime_ms(&path),
        file_count,
    })
}

pub fn write_manifest(manifest: &Rs3CacheManifest, semantic_root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(semantic_root)
        .with_context(|| format!("creating {}", semantic_root.display()))?;
    let path = semantic_root.join(MANIFEST_FILE);
    let json = serde_json::to_string_pretty(manifest)?;
    fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub fn now_rfc3339() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}Z", now.as_secs(), now.subsec_millis())
}
