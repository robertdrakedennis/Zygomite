//! Integration tests for CS2 TypeScript export/transpile (910 fixture).
//!
//! Requires local cache + opcode data. Skips when env paths are missing.
//!
//! Defaults (Alerion workspace):
//! - `RS3_CACHE_DIR=/Users/robert/projects/alerion/cache/unpacked/910`
//! - `RS3_DATA_DIR=/Users/robert/projects/alerion/tools/zwyz-rs3-cache/data`

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

fn run_rs3(subcommand: &str, out_dir: &PathBuf, extra: &[&str]) {
    let (cache, data) = require_fixture().expect("fixture");
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
            subcommand,
            "--out-dir",
            &out_dir.to_string_lossy(),
        ])
        .args(extra)
        .output()
        .expect("run rs3-cache-rs");
    assert!(
        output.status.success(),
        "command failed: {}\nstdout:\n{}\nstderr:\n{}",
        subcommand,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn ts_export_param_count_910() {
    if require_fixture().is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3("ts-export", &out, &[]);
    let params = std::fs::read_to_string(out.join("params.ts")).expect("params.ts");
    assert!(
        params.contains("export const PARAM_COUNT = "),
        "expected PARAM_COUNT in params.ts"
    );
    let count_line = params
        .lines()
        .find(|l| l.contains("PARAM_COUNT"))
        .expect("PARAM_COUNT line");
    let count: u32 = count_line
        .split('=')
        .nth(1)
        .expect("count token")
        .trim()
        .trim_end_matches(';')
        .parse()
        .expect("parse count");
    assert!(count > 8000, "PARAM_COUNT should be ~8060 on 910, got {count}");
}

#[test]
fn ts_export_component_uids_910() {
    if require_fixture().is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3("ts-export", &out, &[]);
    let interfaces = std::fs::read_to_string(out.join("interfaces.ts")).expect("interfaces.ts");
    // interface 517 comp 1 => (517 << 16) | 1 = 33882113
    assert!(
        interfaces.contains("33882113"),
        "expected bank interface UID 33882113 in ComponentId"
    );
    assert!(
        interfaces.contains("Interface_517_Com_1"),
        "expected fallback component name for interface 517 comp 1"
    );
}

#[test]
fn script_signatures_d_ts_910() {
    if require_fixture().is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3("ts-export", &out, &[]);
    let signatures = std::fs::read_to_string(out.join("scripts.d.ts")).expect("scripts.d.ts");
    assert!(
        signatures.contains("export function bank_build_init"),
        "expected named bank_build_init signature in scripts.d.ts"
    );
}

#[test]
fn transpile_bank_script_names_910() {
    if require_fixture().is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3(
        "transpile-scripts",
        &out,
        &["--filter-script", "bank_build", "--max-scripts", "5"],
    );
    let init = std::fs::read_to_string(out.join("bank_build_init.ts")).expect("bank_build_init.ts");
    assert!(
        init.contains("export function bank_build_init"),
        "transpiled bank_build_init should use named export"
    );
    assert!(
        init.contains("Enum_"),
        "expected named enum reference in bank_build_init"
    );
}
