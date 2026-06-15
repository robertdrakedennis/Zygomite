//! Decode every file in the VFX archive (61); report failures.
//! Usage: cargo run --example `scan_vfx` -- <cache-dir>

use rs3_cache_rs::cache::FlatCache;

fn main() -> anyhow::Result<()> {
    let cache_dir = std::env::args().nth(1).expect("usage: <cache-dir>");
    let cache = FlatCache::open(std::path::Path::new(&cache_dir))?;
    let mut groups: Vec<u32> = std::fs::read_dir(std::path::Path::new(&cache_dir).join("61"))?
        .filter_map(|e| {
            e.ok()?
                .path()
                .file_stem()?
                .to_str()?
                .parse::<u32>()
                .ok()
        })
        .collect();
    groups.sort_unstable();
    let mut ok = 0u32;
    let mut failed: Vec<(u32, String)> = Vec::new();
    for group in groups {
        match cache.group_files(61, group) {
            Ok(files) => {
                for (fid, data) in &files {
                    match rs3_cache_rs::vfx::decode(data) {
                        Ok(_) => ok += 1,
                        Err(e) => failed.push((*fid, e.to_string())),
                    }
                }
            }
            Err(e) => failed.push((group, format!("group: {e}"))),
        }
    }
    println!("vfx files: {ok} ok, {} failed", failed.len());
    let mut by_err = std::collections::BTreeMap::new();
    for (_, e) in &failed {
        *by_err.entry(e.clone()).or_insert(0u32) += 1;
    }
    for (e, n) in by_err.iter().take(12) {
        println!("  {n}x {e}");
    }
    Ok(())
}
