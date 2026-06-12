//! Parse every material in archive 26 group 0 and report failures.
//! Usage: cargo run --example scan_materials -- <cache-dir>

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::config::parse_material;

fn main() -> anyhow::Result<()> {
    let cache_dir = std::env::args().nth(1).expect("usage: <cache-dir>");
    let cache = FlatCache::open(std::path::Path::new(&cache_dir))?;
    let files = cache.group_files(26, 0)?;
    let mut ok = 0;
    let mut failed = Vec::new();
    for (id, data) in &files {
        match parse_material(*id, data) {
            Ok(_) => ok += 1,
            Err(e) => failed.push((*id, e.to_string())),
        }
    }
    println!("materials: {} ok, {} failed", ok, failed.len());
    for (id, e) in failed.iter().take(10) {
        println!("  {id}: {e}");
    }
    Ok(())
}
