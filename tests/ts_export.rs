//! Integration tests for CS2 TypeScript export/transpile (910 fixture).
//!
//! Requires local cache + opcode data. Skips when env paths are missing.
//!
//! Defaults (Alerion workspace):
//! - `RS3_CACHE_DIR=/Users/robert/projects/alerion/cache/unpacked/910`
//! - `RS3_DATA_DIR=/Users/robert/projects/alerion/tools/rs3-cache-rs/data`

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn default_cache_dir(build: u32) -> PathBuf {
    PathBuf::from(format!(
        "/Users/robert/projects/alerion/cache/unpacked/{build}"
    ))
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("/Users/robert/projects/alerion/tools/rs3-cache-rs/data")
}

fn default_script_dir_947() -> PathBuf {
    PathBuf::from("/Users/robert/projects/alerion/cache/rs3-cache/947-all/script")
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

fn require_script_fixture_947() -> Option<PathBuf> {
    let script_dir =
        std::env::var_os("RS3_SCRIPT_DIR_947").map_or_else(default_script_dir_947, PathBuf::from);
    if script_dir.is_dir() {
        Some(script_dir)
    } else {
        eprintln!(
            "skip: missing 947 script fixture ({})",
            script_dir.display()
        );
        None
    }
}

fn run_rs3(build: u32, subcommand: &str, out_dir: &Path, extra: &[&str]) {
    let (cache, data) = require_fixture(build).expect("fixture");
    let bin = env!("CARGO_BIN_EXE_rs3-cache-rs");
    let subbuild = if build == 947 { "1" } else { "0" };
    let output = Command::new(bin)
        .args([
            "--cache-dir",
            &cache.to_string_lossy(),
            "--data-dir",
            &data.to_string_lossy(),
            "--build",
            &build.to_string(),
            "--subbuild",
            subbuild,
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

fn ts_export_fixture_910() -> PathBuf {
    static EXPORT_DIR: OnceLock<PathBuf> = OnceLock::new();

    EXPORT_DIR
        .get_or_init(|| {
            let out = std::env::temp_dir()
                .join(format!("rs3-cache-rs-ts-export-910-{}", std::process::id()));
            if out.exists() {
                std::fs::remove_dir_all(&out).expect("clear cached ts-export dir");
            }
            std::fs::create_dir_all(&out).expect("create cached ts-export dir");
            run_rs3(910, "ts-export", &out, &[]);
            out
        })
        .clone()
}

#[test]
fn ts_export_param_count_910() {
    if require_fixture(910).is_none() {
        return;
    }
    let out = ts_export_fixture_910();
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
    assert!(
        count > 8000,
        "PARAM_COUNT should be ~8060 on 910, got {count}"
    );
}

#[test]
fn ts_export_component_uids_910() {
    if require_fixture(910).is_none() {
        return;
    }
    let out = ts_export_fixture_910();
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
    if require_fixture(910).is_none() {
        return;
    }
    let out = ts_export_fixture_910();
    let signatures = std::fs::read_to_string(out.join("scripts.d.ts")).expect("scripts.d.ts");
    assert!(
        signatures.contains("export function bank_build_init")
            || signatures.contains("export function bank_build_scrollbar"),
        "expected named bank build signature in scripts.d.ts"
    );
}

#[test]
fn transpile_bank_script_names_910() {
    if require_fixture(910).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3(
        910,
        "transpile-scripts",
        &out,
        &["--filter-script", "bank_build", "--max-scripts", "5"],
    );
    let script = ["bank_build_init.ts", "bank_build_scrollbar.ts"]
        .iter()
        .find_map(|name| std::fs::read_to_string(out.join(name)).ok())
        .expect("bank build transpile output");
    assert!(
        script.contains("export function bank_build_init")
            || script.contains("export function bank_build_scrollbar"),
        "transpiled bank build script should use named export"
    );
}

#[test]
fn transpile_stockmarket_oninvtransmit_947_imports_script621() {
    let Some(raw_dir) = require_script_fixture_947() else {
        return;
    };
    if require_fixture(947).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3(
        947,
        "transpile-scripts",
        &out,
        &[
            "--filter-script",
            "stockmarket_oninvtransmit",
            "--max-scripts",
            "2",
        ],
    );
    let script = std::fs::read_to_string(out.join("stockmarket_oninvtransmit.ts"))
        .expect("stockmarket_oninvtransmit.ts");
    assert!(
        script.contains("import { script621 } from './script621';"),
        "expected direct script621 import"
    );
    assert!(
        script.contains("export function stockmarket_oninvtransmit(): number"),
        "expected the hook to return script621's value (return script621())"
    );
    assert!(
        script.contains("return script621();"),
        "expected direct script621 tail-return call"
    );
    assert!(
        !script.contains("script_621(pop())"),
        "legacy unresolved call form should be gone"
    );
    let raw = std::fs::read_to_string(raw_dir.join("[clientscript,stockmarket_oninvtransmit].cs2"))
        .expect("raw stockmarket_oninvtransmit");
    assert!(raw.contains("~script621;"), "expected raw parity anchor");
}

#[test]
fn transpile_stockmarket_onvartransmit_947_imports_script621() {
    let Some(raw_dir) = require_script_fixture_947() else {
        return;
    };
    if require_fixture(947).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3(
        947,
        "transpile-scripts",
        &out,
        &[
            "--filter-script",
            "stockmarket_onvartransmit",
            "--max-scripts",
            "2",
        ],
    );
    let script = std::fs::read_to_string(out.join("stockmarket_onvartransmit.ts"))
        .expect("stockmarket_onvartransmit.ts");
    assert!(
        script.contains("import { script621 } from './script621';"),
        "expected direct script621 import"
    );
    assert!(
        script.contains("export function stockmarket_onvartransmit(): number"),
        "expected the hook to return script621's value (return script621())"
    );
    assert!(
        script.contains("return script621();"),
        "expected direct script621 tail-return call"
    );
    let raw = std::fs::read_to_string(raw_dir.join("[clientscript,stockmarket_onvartransmit].cs2"))
        .expect("raw stockmarket_onvartransmit");
    assert!(raw.contains("~script621;"), "expected raw parity anchor");
}

#[test]
fn transpile_stockmarket_onload_947_imports_named_callees() {
    let Some(raw_dir) = require_script_fixture_947() else {
        return;
    };
    if require_fixture(947).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3(
        947,
        "transpile-scripts",
        &out,
        &[
            "--filter-script",
            "stockmarket_onload",
            "--max-scripts",
            "2",
        ],
    );
    let script =
        std::fs::read_to_string(out.join("stockmarket_onload.ts")).expect("stockmarket_onload.ts");
    assert!(
        script.contains("import { script621 } from './script621';"),
        "expected direct import for script621"
    );
    assert!(
        script.contains("import { stockmarket_search_init } from './stockmarket_search_init';"),
        "expected direct import for stockmarket_search_init"
    );
    assert!(
        script.contains("import { script8841 } from './script8841';"),
        "expected direct import for script8841"
    );
    assert!(
        script.contains(
            "UI.Setonvartransmit(callback(\"script588\", [], [varplayerint_135], \"Y\"), 6881479);"
        ),
        "expected decoded var transmit hook callback"
    );
    assert!(
        script.contains("UI.Setonstocktransmit(callback(\"script586\", [], [], \"\"), 6881280);"),
        "expected decoded stock transmit hook callback"
    );
    assert!(
        script.contains(
            "UI.Setonvartransmit(callback(\"script11743\", [], [varplayerint_429, varplayerint_431], \"Y\"), 6881470);"
        ),
        "expected decoded multi-watch var transmit hook callback"
    );
    assert!(
        script.contains(
            "UI.Setoninvtransmit(callback(\"script11743\", [], [inv_540], \"Y\"), 6881470);"
        ),
        "expected decoded inventory transmit hook callback"
    );
    assert!(
        !script.contains("UI.Setonvartransmit(\"Y\","),
        "legacy descriptor-only hook output should be gone"
    );
    let raw = std::fs::read_to_string(raw_dir.join("[clientscript,stockmarket_onload].cs2"))
        .expect("raw stockmarket_onload");
    assert!(
        raw.contains("~script621;"),
        "expected raw script621 call in source"
    );
}

#[test]
fn transpile_script621_947_uses_group_name_and_signature() {
    let Some(raw_dir) = require_script_fixture_947() else {
        return;
    };
    if require_fixture(947).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3(
        947,
        "transpile-scripts",
        &out,
        &["--filter-script", "script621", "--max-scripts", "12"],
    );
    let script = std::fs::read_to_string(out.join("script621.ts")).expect("script621.ts");
    assert!(
        script.contains("// Meta: packed=40697856 group=621 file=0"),
        "expected raw script identity metadata for script621"
    );
    assert!(
        script.contains("export function script621(): number"),
        "expected canonical group-based export name"
    );

    let signatures = std::fs::read_to_string(out.join("scripts.d.ts")).expect("scripts.d.ts");
    assert!(
        signatures.contains("export function script621(): number | void;"),
        "expected canonical script621 declaration in scripts.d.ts"
    );
    let diagnostics = std::fs::read_to_string(out.join("transpile-diagnostics.json"))
        .expect("transpile-diagnostics.json");
    assert!(
        diagnostics.contains("\"build\": 947"),
        "expected diagnostics report for transpile-scripts run"
    );
    let raw = std::fs::read_to_string(raw_dir.join("[proc,script621].cs2")).expect("raw script621");
    assert!(
        raw.contains("[proc,script621]"),
        "expected raw script621 fixture"
    );
}

#[test]
fn transpile_stockmarket_choosecancel_947_disambiguates_collisions() {
    if require_fixture(947).is_none() || require_script_fixture_947().is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().to_path_buf();
    run_rs3(
        947,
        "transpile-scripts",
        &out,
        &[
            "--filter-script",
            "stockmarket_choosecancel",
            "--max-scripts",
            "6",
        ],
    );
    assert!(
        out.join("stockmarket_choosecancel_591.ts").is_file(),
        "expected clientscript collision file"
    );
    assert!(
        out.join("stockmarket_choosecancel_9261.ts").is_file(),
        "expected proc collision file"
    );

    let barrel = std::fs::read_to_string(out.join("scripts.ts")).expect("scripts.ts");
    assert!(
        barrel.contains(
            "export { stockmarket_choosecancel_591 } from './stockmarket_choosecancel_591';"
        ),
        "expected first disambiguated barrel export"
    );
    assert!(
        barrel.contains(
            "export { stockmarket_choosecancel_9261 } from './stockmarket_choosecancel_9261';"
        ),
        "expected second disambiguated barrel export"
    );
}
