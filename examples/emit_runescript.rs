//! G1.2 — read-only `RuneScript` emission over the decompiled corpus.
//!
//! Reads `*.ts` reversible-format files (default `/tmp/948-transpile`), parses each back to the
//! structured IR (`StructuredScript`), and renders it as `RuneScript` with `render_runescript`. The
//! `.ts` editing surface is never touched — this is a presentation-only view used to judge the
//! restyle and surface rough edges before the parser + byte gate land (G1.3).
//!
//! Usage:
//!   cargo run --example `emit_runescript`                       # sweep: parse+render all, report
//!   cargo run --example `emit_runescript` /tmp/948-transpile s  # render script `s` in full

use rs3_cache_rs::transpile::{
    RuneScriptContext, parse_reversible_source, parse_structured_typescript, render_runescript,
};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

fn collect_script_names(dir: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("ts")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.insert(stem.to_string());
        }
    }
    names
}

fn render_file(path: &Path, ctx: &RuneScriptContext) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let parsed = parse_reversible_source(&text).ok()?;
    let script = parse_structured_typescript(&parsed.structured_source).ok()?;
    Some(render_runescript(&script, ctx))
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/948-transpile".to_string());
    let targets: Vec<String> = std::env::args().skip(2).collect();

    let ctx = RuneScriptContext::new(collect_script_names(&dir));

    if !targets.is_empty() {
        for target in &targets {
            let path = Path::new(&dir).join(format!("{target}.ts"));
            println!("// ==== {target} ====");
            match render_file(&path, &ctx) {
                Some(rs) => println!("{rs}"),
                None => eprintln!("  (failed to parse/render {target})"),
            }
        }
        return;
    }

    // Sweep: count how many scripts parse + render, and show a couple of samples.
    let Ok(entries) = fs::read_dir(&dir) else {
        eprintln!("cannot read {dir}");
        return;
    };
    let mut total = 0usize;
    let mut rendered = 0usize;
    let mut parse_fail = 0usize;
    let mut sample_shown = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        total += 1;
        match render_file(&path, &ctx) {
            Some(rs) => {
                rendered += 1;
                if sample_shown < 2 {
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                    eprintln!("// ---- sample: {stem} ----");
                    for line in rs.lines().take(16) {
                        eprintln!("{line}");
                    }
                    eprintln!("// ----");
                    sample_shown += 1;
                }
            }
            None => parse_fail += 1,
        }
    }
    eprintln!("corpus: {total} scripts; rendered {rendered}; parse/render failures {parse_fail}");
}
