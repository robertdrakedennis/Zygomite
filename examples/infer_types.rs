//! Smoke-test G3 type inference on a real decompiled corpus.
//!
//! Reads `*.ts` files (each carrying a `// @cs2` asm trailer) from a directory
//! (default `/tmp/948-transpile`), parses each back to a `CompiledScript`, builds a
//! corpus callee map from script headers, then runs the interprocedural type
//! inference and reports how many scripts were modelled + how many locals refined
//! beyond their raw VM base type.
//!
//! Usage: `cargo run --example infer_types [DIR] [BUILD]`

use rs3_cache_rs::script::{CompiledScript, parse_cs2_asm};
use rs3_cache_rs::transpile::type_constraints::{
    CalleeSig, SignatureTable, infer_program_diag, render_local_type,
};
use rs3_cache_rs::transpile::types::lattice;
use std::collections::HashMap;
use std::fs;

fn meta_script_id(text: &str) -> Option<i32> {
    let key = "\"script_id\":";
    let start = text.find(key)? + key.len();
    let rest = &text[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-')?;
    rest[..end].parse().ok()
}

fn base_unrefined(name: &str) -> bool {
    matches!(
        name,
        "unknown" | "unknown_int" | "unknown_long" | "unknown_object" | "int" | "intarray"
    )
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "/tmp/948-transpile".to_string());
    let build: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(948);
    let sigs = SignatureTable::embedded(build);

    // Parse every script; key by both its packed id and its group id (gosub may use
    // either) so the callee map resolves regardless of id space.
    let mut scripts: Vec<(i32, CompiledScript)> = Vec::new();
    let mut callee_map: HashMap<i32, CalleeSig> = HashMap::new();
    let mut names: HashMap<i32, String> = HashMap::new();
    for entry in fs::read_dir(&dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(script) = parse_cs2_asm(&text) else {
            continue;
        };
        let id = meta_script_id(&text).unwrap_or(-1);
        let sig = CalleeSig {
            arg_int: script.argument_count_int,
            arg_obj: script.argument_count_object,
            arg_long: script.argument_count_long,
            ret_int: 0,
            ret_obj: 0,
            ret_long: 0,
        };
        callee_map.insert(id, sig);
        callee_map.insert(id >> 16, sig); // group id fallback
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            names.insert(id, stem.to_string());
        }
        scripts.push((id, script));
    }
    eprintln!("parsed {} scripts from {dir}", scripts.len());

    let refs: Vec<(i32, &CompiledScript)> = scripts.iter().map(|(id, s)| (*id, s)).collect();
    // `INFER_NO_CALLEE=1` makes any gosub bail (callee=None), isolating the
    // gosub-free subset — used to check whether `conflict`s come from the ret=0
    // gosub approximation (misaligned stack) vs a real stack-discipline bug.
    let no_callee = std::env::var("INFER_NO_CALLEE").is_ok();
    let callee = |id: i32| if no_callee { None } else { callee_map.get(&id).copied() };
    let (result, conflict_hist) = infer_program_diag(&refs, sigs, &callee);

    let total = scripts.len();
    let modelled = result.len();
    let mut total_locals = 0usize;
    let mut refined = 0usize;
    let mut conflicts = 0usize;
    let mut histogram: HashMap<&'static str, usize> = HashMap::new();
    for locals in result.values() {
        for ty in locals.values() {
            total_locals += 1;
            let name = ty.name();
            if name == "conflict" {
                conflicts += 1;
            } else if !base_unrefined(name) {
                refined += 1;
                *histogram.entry(name).or_default() += 1;
            }
        }
    }

    println!(
        "\n=== G3 inference coverage (build {build}{}) ===",
        if no_callee { ", gosub-free only" } else { "" }
    );
    println!(
        "scripts modelled: {modelled}/{total} ({:.1}%)",
        100.0 * modelled as f64 / total.max(1) as f64
    );
    println!(
        "locals refined to a semantic type: {refined}/{total_locals} ({:.1}%)",
        100.0 * refined as f64 / total_locals.max(1) as f64
    );
    println!(
        "locals collapsed to conflict (bad): {conflicts}/{total_locals} ({:.1}%)",
        100.0 * conflicts as f64 / total_locals.max(1) as f64
    );

    let mut top: Vec<(&&str, &usize)> = histogram.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1));
    println!("\ntop inferred semantic types:");
    for (name, count) in top.into_iter().take(20) {
        println!("  {count:>6}  {name}");
    }

    println!("\ntop conflict-causing type pairs (meet → conflict):");
    for ((a, b), count) in conflict_hist.into_iter().take(15) {
        println!("  {count:>6}  {} × {}", a.name(), b.name());
    }

    // Showcase: the modelled script with the most semantically-refined locals, so the
    // before/after is always a meaningful demonstration.
    let showcase = result
        .iter()
        .map(|(id, locals)| {
            let refined = locals.values().filter(|t| !base_unrefined(t.name())).count();
            (id, refined)
        })
        .filter(|(_, refined)| *refined >= 2)
        .max_by_key(|(_, refined)| *refined)
        .map(|(id, _)| (id, names.get(id).map_or("<script>", |s| s.as_str())));
    if let Some((id, name)) = showcase {
        let locals = &result[id];
        println!("\n=== {name}: rendered locals (G3.3 policy) ===");
        println!("  {:<22} {:<14} -> {}", "slot", "today", "with G3");
        let mut entries: Vec<_> = locals.iter().collect();
        entries.sort_by_key(|((d, i), _)| (format!("{d:?}"), *i));
        let unknown = lattice().wk().unknown;
        for ((domain, index), ty) in entries.into_iter().take(30) {
            let base = render_local_type(unknown, *domain); // today's base VM type
            let rendered = render_local_type(*ty, *domain);
            let marker = if rendered == base { "" } else { "  <-- refined" };
            println!("  $local_{domain:?}_{index:<10} def_{base:<10} def_{rendered}{marker}");
        }
    }
}
