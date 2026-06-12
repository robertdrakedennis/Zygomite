//! Dump raw bytes of one material config file (archive 26, group 0) from a
//! flat cache, as hex on stdout. Used to diff material formats across builds.
//!
//! Usage: cargo run --example dump_material_raw -- <cache-dir> <material-id>

use rs3_cache_rs::cache::FlatCache;

fn main() -> anyhow::Result<()> {
    let cache_dir = std::env::args().nth(1).expect("usage: <cache-dir> <id>");
    let id: u32 = std::env::args()
        .nth(2)
        .expect("usage: <cache-dir> <id>")
        .parse()?;
    let cache = FlatCache::open(std::path::Path::new(&cache_dir))?;
    let files = cache.group_files(26, 0)?;
    let data = files
        .get(&id)
        .ok_or_else(|| anyhow::anyhow!("material {id} not in group"))?;
    println!("len={}", data.len());
    for chunk in data.chunks(16) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        println!("{}", hex.join(" "));
    }
    Ok(())
}
