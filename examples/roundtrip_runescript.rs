//! G1.3 — corpus round-trip verification for the `RuneScript` surface.
//!
//! For every `*.ts` reversible script: parse it to the structured IR, render `RuneScript`, parse the
//! `RuneScript` back, and re-render. If the parser faithfully inverts the emitter, the two `RuneScript`
//! texts are identical (emit∘parse∘emit == emit). This is the presentation-level proof that the
//! `RuneScript` surface round-trips; the authoritative byte gate lands when the surface is wired into
//! the transpile CLI (G1.4), reusing the existing `ReverseCompileContext`.
//!
//! Usage: cargo run --example `roundtrip_runescript` [DIR]

use rs3_cache_rs::transpile::{
    RuneScriptContext, parse_reversible_source, parse_runescript, parse_structured_typescript,
    render_runescript,
};
use std::collections::HashSet;
use std::fs;

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

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/948-transpile".to_string());
    let ctx = RuneScriptContext::new(collect_script_names(&dir));

    let Ok(entries) = fs::read_dir(&dir) else {
        eprintln!("cannot read {dir}");
        return;
    };

    let mut scripts = 0usize; // reversible scripts (parse_structured_typescript ok)
    let mut idempotent = 0usize; // emit∘parse∘emit == emit
    let mut parse_fail = 0usize; // rs_parse rejected the rendered RuneScript
    let mut mismatch = 0usize; // re-render differed
    let mut parse_fail_samples: Vec<String> = Vec::new();
    let mut mismatch_samples: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = parse_reversible_source(&text) else {
            continue;
        };
        let Ok(script) = parse_structured_typescript(&parsed.structured_source) else {
            continue;
        };
        scripts += 1;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");

        let rs1 = render_runescript(&script, &ctx);
        match parse_runescript(&rs1, &ctx) {
            Ok(reparsed) => {
                let rs2 = render_runescript(&reparsed, &ctx);
                if rs1 == rs2 {
                    idempotent += 1;
                } else {
                    mismatch += 1;
                    if mismatch_samples.len() < 5 {
                        mismatch_samples.push(format!("{stem}: {}", first_diff(&rs1, &rs2)));
                    }
                }
            }
            Err(err) => {
                parse_fail += 1;
                if parse_fail_samples.len() < 8 {
                    parse_fail_samples.push(format!("{stem}: {err}"));
                }
            }
        }
    }

    eprintln!("RuneScript round-trip over {scripts} scripts:");
    eprintln!("  idempotent (emit==reparse-emit): {idempotent}");
    eprintln!("  parse failures:                  {parse_fail}");
    eprintln!("  re-render mismatches:            {mismatch}");
    if !parse_fail_samples.is_empty() {
        eprintln!("-- parse-failure samples --");
        for s in &parse_fail_samples {
            eprintln!("  {s}");
        }
    }
    if !mismatch_samples.is_empty() {
        eprintln!("-- mismatch samples (first differing line) --");
        for s in &mismatch_samples {
            eprintln!("  {s}");
        }
    }
}

/// Return the first differing line between two renders, for triage.
fn first_diff(a: &str, b: &str) -> String {
    for (la, lb) in a.lines().zip(b.lines()) {
        if la != lb {
            return format!("{la:?}  !=  {lb:?}");
        }
    }
    "(differ in length only)".to_string()
}
