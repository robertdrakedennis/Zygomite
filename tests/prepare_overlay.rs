//! Integration tests for `prepare-overlay` (910/947 fixture).

use std::path::PathBuf;
use std::process::Command;

fn default_cache_dir() -> PathBuf {
    PathBuf::from("/Users/robert/projects/alerion/cache/unpacked/910")
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("/Users/robert/projects/alerion/tools/zwyz-rs3-cache/data")
}

fn cache_dir() -> PathBuf {
    std::env::var_os("RS3_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_cache_dir)
}

fn data_dir() -> PathBuf {
    std::env::var_os("RS3_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_data_dir)
}

fn require_fixture() -> Option<(PathBuf, PathBuf)> {
    let cache = cache_dir();
    let data = data_dir();
    if cache.is_dir() && data.is_dir() {
        Some((cache, data))
    } else {
        eprintln!(
            "skip: missing fixture (cache={}, data={})",
            cache.display(),
            data.display()
        );
        None
    }
}

#[test]
fn prepare_overlay_writes_manifest_and_refs_910() {
    let Some((cache, data)) = require_fixture() else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    let bin = env!("CARGO_BIN_EXE_rs3-cache-rs");
    let output = Command::new(bin)
        .args([
            "--cache-dir",
            &cache.to_string_lossy(),
            "--data-dir",
            &data.to_string_lossy(),
            "--build",
            "910",
            "--subbuild",
            "0",
            "prepare-overlay",
            "--out-dir",
            &out.to_string_lossy(),
        ])
        .output()
        .expect("run prepare-overlay");
    assert!(
        output.status.success(),
        "prepare-overlay failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest_path = out.join(".rs3-cache-manifest.json");
    assert!(manifest_path.is_file(), "missing manifest");
    let manifest = std::fs::read_to_string(&manifest_path).expect("read manifest");
    assert!(manifest.contains("\"tool_version\""));
    assert!(manifest.contains("\"cache_fingerprint\""));

    let refs_obj = out.join("refs/obj.json");
    assert!(refs_obj.is_file(), "missing refs/obj.json");
    let refs = std::fs::read_to_string(&refs_obj).expect("read refs");
    assert!(refs.len() > 10, "obj refs should be non-empty");

    assert!(out.join("raw-flat").is_dir(), "missing raw-flat");
}
