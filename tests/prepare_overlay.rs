//! Integration tests for `prepare-overlay` fixtures.

use std::path::PathBuf;
use std::process::Command;

fn default_cache_dir(build: u32) -> PathBuf {
    PathBuf::from(format!(
        "/Users/robert/projects/alerion/cache/unpacked/{build}"
    ))
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("/Users/robert/projects/alerion/tools/rs3-cache-rs/data")
}

fn cache_dir(build: u32) -> PathBuf {
    let scoped_key = format!("RS3_CACHE_DIR_{build}");
    std::env::var_os(&scoped_key)
        .or_else(|| std::env::var_os("RS3_CACHE_DIR"))
        .map_or_else(|| default_cache_dir(build), PathBuf::from)
}

fn data_dir() -> PathBuf {
    std::env::var_os("RS3_DATA_DIR").map_or_else(default_data_dir, PathBuf::from)
}

fn require_fixture(build: u32) -> Option<(PathBuf, PathBuf)> {
    let cache = cache_dir(build);
    let data = data_dir();
    if cache.is_dir() && data.is_dir() {
        Some((cache, data))
    } else {
        eprintln!(
            "skip: missing fixture build={build} (cache={}, data={})",
            cache.display(),
            data.display()
        );
        None
    }
}

fn run_prepare_overlay(build: u32, archives: Option<&str>) {
    let Some((cache, data)) = require_fixture(build) else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    let bin = env!("CARGO_BIN_EXE_rs3-cache-rs");
    let mut command = Command::new(bin);
    command.args([
        "--cache-dir",
        &cache.to_string_lossy(),
        "--data-dir",
        &data.to_string_lossy(),
        "--build",
        &build.to_string(),
        "--subbuild",
        "0",
        "prepare-overlay",
        "--out-dir",
        &out.to_string_lossy(),
    ]);
    if let Some(archives) = archives {
        command.args(["--archives", archives]);
    }
    let output = command.output().expect("run prepare-overlay");
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

    let deps_components = out.join("deps/components.json");
    assert!(deps_components.is_file(), "missing deps/components.json");
    let deps_coverage = out.join("deps/coverage.json");
    assert!(deps_coverage.is_file(), "missing deps/coverage.json");
    let deps_scripts = out.join("deps/scripts.jsonl");
    assert!(deps_scripts.is_file(), "missing deps/scripts.jsonl");

    assert!(out.join("raw-flat").is_dir(), "missing raw-flat");
}

#[test]
fn prepare_overlay_writes_manifest_and_refs_910() {
    run_prepare_overlay(910, Some("12"));
}

#[test]
fn prepare_overlay_writes_manifest_and_refs_947() {
    run_prepare_overlay(947, Some("12"));
}
